use std::collections::HashSet;
use std::rc::Rc;
use std::time::Duration;

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{App, AsyncApp, Context, Entity, Task};
use rise_canon::RiseGuideSignal;
use rise_canon::rise_key::bot as bot_key;
use rise_canon::rise_key::user as user_key;
use rise_canon::{rise_request, rise_wire_methods};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::CancellationError;
use rise_engine::rise_upload::rise_url_session::{URLError, URLErrorCode};
use rise_i18n::tr;
use rise_platform::haptics::{NotificationKind, notification};
use rise_platform::contacts;
use serde_json::Value;

use crate::core::desktop_platform::rise_app_events::RiseAppEvents;
use crate::core::guides::guide_signal_bus::GuideSignalBus;
use crate::core::lib::any_dictionary_bridge::AnyDictionaryBridge;
use crate::core::lib::canon_errors::CanonErrors;
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::core::utilities::sai_cache::SaiCache;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiSliceState;
use crate::core::lib::sai::core::websocket::sai_web_socket_manager::SAI_WS_DID_DISCONNECT;
use crate::core::lib::sai::core::websocket::sai_ws_deferred_outbound_coordinator::SaiWsDeferredOutboundCoordinator;
use crate::core::lib::sai::modules::sai_debouncer::sai_debouncer::SaiDebouncer;
use crate::core::lib::sai::modules::sai_updater::sai_updater::{SaiUpdater, SaiUpdaterWriteScope};
use crate::core::lib::sai::modules::sai_ws::sai_ws::SaiWs;
use crate::core::lib::sai::modules::sai_ws::sai_ws_optimistic_update::SaiWsOptimisticLocalFile;
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{
    SaiWsCachePreferredLayer, SaiWsDiskCachePolicy, SaiWsSendError,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws_response_cache::SaiWsResponseCache;
use crate::core::lib::sai::sai::Sai;
use crate::core::rise_engine_bridge::rise_engine_app_runtime::{RISE_ENGINE_CONNECTION_READY, RiseEngineAppRuntime};
use crate::core::rise_engine_bridge::rise_transport_status::RiseTransportStatus;
use crate::core::widgets::notifier::notifier_store::{
    Notifier, NotifierIcon, NotifierPosition, NotifierSendOptions, NotifierType,
};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::stores::chats::chat_actions::chat_actions::ChatActionsStore;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatInfo, ChatType};
use crate::modules::chat::stores::chats::chat_interactions::chat_interactions::ChatInteractionsStore;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::user::engine::core::rise_user_block_wire_models::RiseBlockScope;
use crate::modules::user::engine::rise_user_block_presentation::RiseUserBlockLoadState;
use crate::modules::user::shared::contact_device_book::{ContactDeviceAuthorization, ContactDeviceBook};
use crate::modules::user::stores::user::user_actions::user_actions::UserActionsStore;
use crate::modules::user::stores::user::user_actions::user_contact_types::{
    UserContactBlockFact, UserContactDraft, UserContactLimits, UserContactMutationError, UserContactPeer,
    UserContactPhotoUpload,
};
use crate::modules::user::stores::user::user_actions::user_contact_wire_types::{
    SyncUserContactsGatewayDTO, UserContactPhotoSuggestionWireDTO, UserSavedContactWireDTO,
};
use crate::modules::user::stores::user::user_interactions::user_interactions::UserInteractionsStore;
use crate::modules::user::stores::user::user_services::user_guide_sandbox_hooks::user_guide_sandbox_hooks;
use crate::modules::user::stores::user::user_services::user_services::{
    UserServicesStore, UserServicesStoreContactForegroundRefresh,
};
use crate::modules::user::stores::user::user_services::user_services_hooks::UserServicesHooks;
use crate::modules::user::stores::user_profile_photos::user_profile_photos_interactions::user_profile_photos_interactions::UserProfilePhotosInteractionsStore;

type UserContactResultHandler<T> = Rc<dyn Fn(anyhow::Result<T>, &mut App)>;

struct UserContactSyncRunFinish {
    run_id: Uuid,
    cx: AsyncApp,
}

impl Drop for UserContactSyncRunFinish {
    fn drop(&mut self) {
        let run_id = self.run_id;
        let cx = self.cx.clone();
        self.cx
            .foreground_executor()
            .spawn(async move {
                cx.update(|cx| {
                    UserInteractionsStore::shared(cx)
                        .update(cx, |store, cx| store.finish_address_book_sync_run(run_id, cx));
                });
            })
            .detach();
    }
}

impl UserInteractionsStore {
    pub fn bootstrap_contacts_for_authenticated_account(&mut self, cx: &mut Context<Self>) {
        if !Self::contact_is_authenticated(cx) {
            return;
        }
        let Some(account_id) = Self::contact_effective_user_id(cx) else {
            return;
        };
        Self::contact_service(cx)
            .update(cx, |service, cx| service.purge_legacy_saved_contacts_disk_cache(&account_id, cx));
        self.install_contact_observers_if_needed(cx);
        self.refresh_contact_authorization(cx);
        let ready = RiseEngineAppRuntime::shared(cx).update(cx, |runtime, cx| runtime.ready_connection(&account_id, cx));
        cx.spawn(async move |this, cx| {
            if ready.await.is_none() {
                return;
            }
            let _ = this.update(cx, |store, cx| {
                if !Self::is_active_contact_account(&account_id, cx) {
                    return;
                }
                store.ensure_contact_sync_settings_loaded(cx);
                if Self::claim_contact_foreground_refresh(
                    UserServicesStoreContactForegroundRefresh::SavedContactFolders,
                    cx,
                ) {
                    store.load_saved_contacts_for_chat_folders(cx);
                }
                if Self::claim_contact_foreground_refresh(UserServicesStoreContactForegroundRefresh::JoinEvents, cx) {
                    Self::list_contact_join_events(cx);
                }
                if Self::claim_contact_foreground_refresh(
                    UserServicesStoreContactForegroundRefresh::PhotoSuggestions,
                    cx,
                ) {
                    Self::contact_actions(cx)
                        .update(cx, |actions, cx| actions.list_contact_photo_suggestions_action(None, 0, cx));
                }
            });
        })
        .detach();
    }

    pub fn prepare_contact_sync_after_registration(&mut self, cx: &mut Context<Self>) {
        if !Self::contact_is_authenticated(cx) {
            return;
        }
        self.install_contact_observers_if_needed(cx);
        self.refresh_contact_authorization(cx);
        match Self::contact_authorization(cx) {
            ContactDeviceAuthorization::Authorized | ContactDeviceAuthorization::Limited => {
                self.update_contact_sync_preference(
                    true,
                    Some(Box::new(|success, cx: &mut App| {
                        if success {
                            Self::with_contact_store(cx, |store, cx| store.synchronize_address_book(cx));
                        }
                    })),
                    cx,
                );
            }
            ContactDeviceAuthorization::NotDetermined => {
                Self::contact_service(cx).update(cx, |service, cx| {
                    service.should_present_contact_sync_onboarding = true;
                    cx.notify();
                });
            }
            ContactDeviceAuthorization::Denied | ContactDeviceAuthorization::Restricted => {
                if contacts::contacts_support().is_supported() {
                    self.update_contact_sync_preference(false, None, cx);
                }
            }
        }
    }

    pub fn accept_contact_sync_onboarding(&mut self, cx: &mut Context<Self>) {
        let Some(account_id) = Self::contact_effective_user_id(cx) else {
            return;
        };
        Self::contact_service(cx).update(cx, |service, cx| {
            service.should_present_contact_sync_onboarding = false;
            cx.notify();
        });
        cx.spawn(async move |this, cx| {
            let requested = ContactDeviceBook::shared().request_authorization().await;
            let _ = this.update(cx, |store, cx| {
                let authorization = match requested {
                    Ok(authorization) => authorization,
                    Err(error) => {
                        if Self::is_contact_cancellation(&error) {
                            return;
                        }
                        store.update_contact_sync_preference(false, None, cx);
                        Self::send_contact_notifier(
                            tr("contact_sync_permission_failed"),
                            Some(error.to_string()),
                            NotifierType::Error,
                            NotifierIcon::None,
                            cx,
                        );
                        Self::contact_service(cx)
                            .update(cx, |service, cx| service.present_next_contact_photo_suggestion_if_needed(cx));
                        return;
                    }
                };
                if !Self::is_active_contact_account(&account_id, cx) {
                    return;
                }
                Self::set_contact_authorization(authorization, cx);
                if authorization.can_read_and_write() {
                    store.update_contact_sync_preference(
                        true,
                        Some(Box::new(|success, cx: &mut App| {
                            if success {
                                Self::with_contact_store(cx, |store, cx| store.synchronize_address_book(cx));
                            }
                        })),
                        cx,
                    );
                } else {
                    store.update_contact_sync_preference(false, None, cx);
                    store.present_contact_permission_denied(cx);
                }
                Self::contact_service(cx)
                    .update(cx, |service, cx| service.present_next_contact_photo_suggestion_if_needed(cx));
            });
        })
        .detach();
    }

    pub fn decline_contact_sync_onboarding(&mut self, cx: &mut Context<Self>) {
        Self::contact_service(cx).update(cx, |service, cx| {
            service.should_present_contact_sync_onboarding = false;
            cx.notify();
        });
        self.update_contact_sync_preference(false, None, cx);
        Self::contact_service(cx)
            .update(cx, |service, cx| service.present_next_contact_photo_suggestion_if_needed(cx));
    }

    pub fn set_contact_sync_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if !enabled {
            let account_id = Self::contact_effective_user_id(cx);
            Self::contact_service(cx).update(cx, |service, cx| {
                service.contact_sync_enable_after_system_settings = false;
                cx.notify();
            });
            Self::cancel_contact_sync_run(cx);
            if let Some(account_id) = account_id.clone() {
                Self::contact_service(cx)
                    .update(cx, |service, cx| service.clear_contact_join_local_data(&account_id, cx));
                Self::clear_address_book_snapshot_in_background(account_id, cx);
            }
            self.update_contact_sync_preference(
                false,
                Some(Box::new(move |success, cx: &mut App| {
                    Self::with_contact_store(cx, move |store, cx| {
                        let Some(account_id) = account_id else {
                            return;
                        };
                        if !Self::is_active_contact_account(&account_id, cx) {
                            return;
                        }
                        Self::list_contact_join_events(cx);
                        if !success
                            && Self::contact_sync_enabled(cx)
                            && Self::contact_authorization(cx).can_read_and_write()
                        {
                            store.synchronize_address_book(cx);
                        }
                    });
                })),
                cx,
            );
            return;
        }

        self.refresh_contact_authorization(cx);
        match Self::contact_authorization(cx) {
            ContactDeviceAuthorization::Authorized | ContactDeviceAuthorization::Limited => {
                self.update_contact_sync_preference(
                    true,
                    Some(Box::new(|success, cx: &mut App| {
                        if success {
                            Self::with_contact_store(cx, |store, cx| store.synchronize_address_book(cx));
                        }
                    })),
                    cx,
                );
            }
            ContactDeviceAuthorization::NotDetermined => {
                let Some(account_id) = Self::contact_effective_user_id(cx) else {
                    return;
                };
                cx.spawn(async move |this, cx| {
                    let requested = ContactDeviceBook::shared().request_authorization().await;
                    let _ = this.update(cx, |store, cx| {
                        let authorization = match requested {
                            Ok(authorization) => authorization,
                            Err(error) => {
                                if Self::is_contact_cancellation(&error) {
                                    return;
                                }
                                Self::send_contact_notifier(
                                    tr("contact_sync_permission_failed"),
                                    Some(error.to_string()),
                                    NotifierType::Error,
                                    NotifierIcon::None,
                                    cx,
                                );
                                return;
                            }
                        };
                        if !Self::is_active_contact_account(&account_id, cx) {
                            return;
                        }
                        Self::set_contact_authorization(authorization, cx);
                        if authorization.can_read_and_write() {
                            store.update_contact_sync_preference(
                                true,
                                Some(Box::new(|success, cx: &mut App| {
                                    if success {
                                        Self::with_contact_store(cx, |store, cx| store.synchronize_address_book(cx));
                                    }
                                })),
                                cx,
                            );
                        } else {
                            store.update_contact_sync_preference(false, None, cx);
                            store.present_contact_permission_denied(cx);
                        }
                    });
                })
                .detach();
            }
            ContactDeviceAuthorization::Denied => {
                Self::contact_service(cx).update(cx, |service, cx| {
                    service.contact_sync_enable_after_system_settings = true;
                    cx.notify();
                });
                self.present_contact_permission_denied(cx);
            }
            ContactDeviceAuthorization::Restricted => {
                Self::send_contact_notifier(
                    tr("contact_sync_restricted_title"),
                    Some(tr("contact_sync_restricted_description")),
                    NotifierType::Warning,
                    NotifierIcon::None,
                    cx,
                );
            }
        }
    }

    pub fn retry_contact_sync_settings(&mut self, cx: &mut Context<Self>) {
        Self::contact_service(cx).update(cx, |service, cx| {
            service.contact_sync_settings_error = None;
            cx.notify();
        });
        self.reload_contact_sync_settings(cx);
    }

    fn reload_contact_sync_settings(&mut self, cx: &mut Context<Self>) {
        Self::contact_actions(cx).update(cx, |actions, cx| {
            actions.get_contact_sync_settings_action(
                Some(Box::new(|result, cx: &mut App| {
                    if result.is_ok() {
                        Self::with_contact_store(cx, |store, cx| store.handle_loaded_contact_sync_settings(cx));
                    }
                })),
                cx,
            );
        });
    }

    pub fn open_contact_system_settings(&mut self, cx: &mut Context<Self>) {
        if !contacts::open_settings(cx).is_supported() {
            Self::send_contact_notifier(
                tr("contact_sync_restricted_title"),
                Some(tr("contact_sync_restricted_description")),
                NotifierType::Warning,
                NotifierIcon::None,
                cx,
            );
        }
    }

    pub fn application_did_become_active_for_contacts(&mut self, cx: &mut Context<Self>) {
        if !Self::contact_is_authenticated(cx) {
            return;
        }
        let previous = Self::contact_authorization(cx);
        self.refresh_contact_authorization(cx);
        let current = Self::contact_authorization(cx);
        if previous.can_read_and_write() && !current.can_read_and_write() && Self::contact_sync_enabled(cx) {
            self.set_contact_sync_enabled(false, cx);
        }
        let enable_after_system_settings =
            Self::contact_service(cx).read(cx).contact_sync_enable_after_system_settings;
        let current = Self::contact_authorization(cx);
        if enable_after_system_settings && !previous.can_read_and_write() && current.can_read_and_write() {
            Self::contact_service(cx).update(cx, |service, cx| {
                service.contact_sync_enable_after_system_settings = false;
                cx.notify();
            });
            self.update_contact_sync_preference(
                true,
                Some(Box::new(|success, cx: &mut App| {
                    if success {
                        Self::with_contact_store(cx, |store, cx| store.synchronize_address_book(cx));
                    }
                })),
                cx,
            );
            return;
        }
        if Self::contact_authorization(cx).can_read_and_write()
            && Self::claim_contact_foreground_refresh(UserServicesStoreContactForegroundRefresh::AddressBookSync, cx)
        {
            self.reload_contact_sync_settings(cx);
        }
        if Self::claim_contact_foreground_refresh(UserServicesStoreContactForegroundRefresh::JoinEvents, cx) {
            Self::list_contact_join_events(cx);
        }
        if Self::claim_contact_foreground_refresh(UserServicesStoreContactForegroundRefresh::PhotoSuggestions, cx) {
            Self::contact_actions(cx).update(cx, |actions, cx| actions.list_contact_photo_suggestions_action(None, 0, cx));
        }
        if Self::claim_contact_foreground_refresh(UserServicesStoreContactForegroundRefresh::SavedContactFolders, cx) {
            self.load_saved_contacts_for_chat_folders(cx);
        }
    }

    pub fn refresh_saved_contact(&mut self, target_user_id: &str, cx: &mut Context<Self>) {
        if target_user_id.is_empty() || Self::contact_actions(cx).read(cx).get_saved_contact_sai.is_pending() {
            return;
        }
        if (user_guide_sandbox_hooks().owns_user_id)(target_user_id, cx) {
            return;
        }
        let target_user_id = target_user_id.to_owned();
        Self::contact_service(cx).update(cx, |service, cx| {
            service.saved_contact_failed_targets.remove(&target_user_id);
            cx.notify();
        });
        let target = target_user_id.clone();
        Self::contact_actions(cx).update(cx, |actions, cx| actions.get_saved_contact_action(&target, None, cx));
        self.refresh_foreign_profile_user_id(target_user_id, cx);
    }

    pub fn save_contact(
        &mut self,
        draft: UserContactDraft,
        peer: UserContactPeer,
        on_result: impl Fn(anyhow::Result<UserSavedContactWireDTO>, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        let on_result: UserContactResultHandler<UserSavedContactWireDTO> = Rc::new(on_result);
        let account_id = Self::contact_effective_user_id(cx);
        let pending = Self::contact_service(cx).read(cx).saved_contact_pending_targets.contains(&draft.target_user_id);
        let Some(account_id) = account_id.filter(|_| !pending) else {
            on_result(Err(UserContactMutationError::InvalidContact.into()), cx);
            return;
        };
        let previous =
            Self::contact_service(cx).update(cx, |service, cx| service.saved_contact(&draft.target_user_id, cx));
        let sync_to_phone = draft.sync_to_phone;
        let begin_draft = draft.clone();
        let begin_peer = peer.clone();
        let begin_account_id = account_id.clone();
        let begin_on_result = Rc::clone(&on_result);
        let begin_server_save = move |with_device_book: bool, cx: &mut App| {
            let draft = begin_draft;
            let peer = begin_peer;
            let account_id = begin_account_id;
            let on_result = begin_on_result;
            if !Self::is_active_contact_account(&account_id, cx) {
                on_result(Err(CancellationError.into()), cx);
                return;
            }
            if Self::contact_service(cx).read(cx).saved_contact_pending_targets.contains(&draft.target_user_id) {
                on_result(Err(UserContactMutationError::InvalidContact.into()), cx);
                return;
            }
            let action_draft = draft.clone();
            let action_peer = peer.clone();
            Self::contact_actions(cx).update(cx, |actions, cx| {
                actions.upsert_saved_contact_action(
                    action_draft,
                    action_peer,
                    None,
                    Box::new(move |result, cx: &mut App| {
                        Self::with_contact_store(cx, move |store, cx| match result {
                            Ok(contact) => {
                                if !Self::is_active_contact_account(&account_id, cx) {
                                    on_result(Err(CancellationError.into()), cx);
                                    return;
                                }
                                if !draft.sync_to_phone || !with_device_book {
                                    store.finish_contact_save_success(contact, &on_result, cx);
                                    return;
                                }
                                store.save_contact_to_phone_after_server_save(
                                    contact, draft, peer, previous, account_id, on_result, cx,
                                );
                            }
                            Err(error) => store.finish_contact_save_failure(error, &on_result, cx),
                        });
                    }),
                    cx,
                );
            });
        };

        if !sync_to_phone {
            begin_server_save(false, cx);
            return;
        }
        let authorization = self.ensure_contact_write_authorization(cx);
        cx.spawn(async move |this, cx| {
            let outcome: anyhow::Result<()> = async {
                let authorization = authorization.await?;
                if !cx.update(|cx| Self::is_active_contact_account(&account_id, cx)) {
                    return Err(CancellationError.into());
                }
                if !authorization.can_read_and_write() {
                    return Err(UserContactMutationError::ContactsPermissionUnavailable.into());
                }
                let book_account_id = account_id.clone();
                let target_user_id = draft.target_user_id.clone();
                let phone = peer.phone.clone();
                let profile_tag = peer.tag.clone();
                let can_save = cx
                    .background_executor()
                    .spawn(async move {
                        ContactDeviceBook::shared()
                            .can_save_peer_contact(&book_account_id, &target_user_id, phone.as_deref(), &profile_tag)
                            .await
                    })
                    .await;
                if !can_save {
                    return Err(UserContactMutationError::PhoneUnavailable.into());
                }
                Ok(())
            }
            .await;
            let _ = this.update(cx, |_, cx| match outcome {
                Ok(()) => begin_server_save(true, cx),
                Err(error) if Self::is_contact_cancellation(&error) => on_result(Err(CancellationError.into()), cx),
                Err(error) => {
                    Sai::log(
                        "User",
                        move || format!("contact device book unavailable, server only save: {error}"),
                        SaiLogKind::Network,
                        cx,
                    );
                    begin_server_save(false, cx);
                }
            });
        })
        .detach();
    }

    #[allow(clippy::too_many_arguments)]
    fn save_contact_to_phone_after_server_save(
        &mut self,
        contact: UserSavedContactWireDTO,
        draft: UserContactDraft,
        peer: UserContactPeer,
        previous: Option<UserSavedContactWireDTO>,
        account_id: String,
        on_result: UserContactResultHandler<UserSavedContactWireDTO>,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |this, cx| {
            let outcome: anyhow::Result<()> = async {
                if !cx.update(|cx| Self::is_active_contact_account(&account_id, cx)) {
                    return Err(CancellationError.into());
                }
                let book_account_id = account_id.clone();
                let target_user_id = draft.target_user_id.clone();
                let first_name = draft.normalized_first_name();
                let last_name = draft.normalized_last_name();
                let phone = peer.phone.clone();
                let profile_tag = peer.tag.clone();
                cx.background_executor()
                    .spawn(async move {
                        ContactDeviceBook::shared()
                            .save_peer_contact(
                                &book_account_id,
                                &target_user_id,
                                &first_name,
                                &last_name,
                                phone.as_deref(),
                                &profile_tag,
                                None,
                            )
                            .await
                    })
                    .await?;
                if !cx.update(|cx| Self::is_active_contact_account(&account_id, cx)) {
                    return Err(CancellationError.into());
                }
                Ok(())
            }
            .await;
            let _ = this.update(cx, |store, cx| match outcome {
                Ok(()) => store.finish_contact_save_success(contact, &on_result, cx),
                Err(error) if Self::is_contact_cancellation(&error) => on_result(Err(CancellationError.into()), cx),
                Err(error) => store.compensate_failed_contact_phone_save(
                    previous,
                    peer,
                    draft.target_user_id,
                    account_id,
                    error,
                    on_result,
                    cx,
                ),
            });
        })
        .detach();
    }

    pub fn delete_contact(
        &mut self,
        target_user_id: &str,
        on_result: impl Fn(anyhow::Result<()>, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        let on_result: UserContactResultHandler<()> = Rc::new(on_result);
        let account_id = Self::contact_effective_user_id(cx);
        let pending = Self::contact_service(cx).read(cx).saved_contact_pending_targets.contains(target_user_id);
        let Some(account_id) = account_id.filter(|_| !pending) else {
            on_result(Err(UserContactMutationError::InvalidContact.into()), cx);
            return;
        };
        let target_user_id = target_user_id.to_owned();
        let action_target = target_user_id.clone();
        Self::contact_actions(cx).update(cx, |actions, cx| {
            actions.delete_saved_contact_action(
                &action_target,
                None,
                Box::new(move |result, cx: &mut App| {
                    Self::with_contact_store(cx, move |_, cx| match result {
                        Ok(()) => {
                            if !Self::is_active_contact_account(&account_id, cx) {
                                on_result(Err(CancellationError.into()), cx);
                                return;
                            }
                            let peer = Self::contact_service(cx).update(cx, |service, cx| {
                                service
                                    .saved_contact_gateway(&target_user_id, cx)
                                    .and_then(|gateway| gateway.contact)
                                    .and_then(|contact| contact.user)
                            });
                            if let Some(peer) = peer
                                && peer.id == target_user_id
                                && !peer.logo.is_empty()
                            {
                                ChatInteractionsStore::shared(cx).update(cx, |chat, cx| {
                                    chat.apply_peer_logo(&target_user_id, &peer.logo, cx);
                                });
                            }
                            cx.spawn(async move |_, cx| {
                                if !cx.update(|cx| Self::is_active_contact_account(&account_id, cx)) {
                                    cx.update(|cx| on_result(Err(CancellationError.into()), cx));
                                    return;
                                }
                                let book_account_id = account_id.clone();
                                let book_target_user_id = target_user_id.clone();
                                let cleanup = cx
                                    .background_executor()
                                    .spawn(async move {
                                        ContactDeviceBook::shared()
                                            .delete_mapped_peer_contact(&book_account_id, &book_target_user_id)
                                            .await
                                    })
                                    .await;
                                let phone_cleanup_failed = match cleanup {
                                    Ok(()) => false,
                                    Err(error) if Self::is_contact_cancellation(&error) => {
                                        cx.update(|cx| on_result(Err(CancellationError.into()), cx));
                                        return;
                                    }
                                    Err(_) => true,
                                };
                                cx.update(|cx| {
                                    notification(NotificationKind::Success);
                                    Self::send_contact_notifier(
                                        tr("contact_deleted_notification"),
                                        phone_cleanup_failed.then(|| tr("contact_deleted_phone_cleanup_failed")),
                                        if phone_cleanup_failed { NotifierType::Warning } else { NotifierType::Success },
                                        NotifierIcon::None,
                                        cx,
                                    );
                                    on_result(Ok(()), cx);
                                });
                            })
                            .detach();
                        }
                        Err(error) => {
                            notification(NotificationKind::Error);
                            Self::send_contact_notifier(
                                tr("contact_delete_failed"),
                                Some(CanonErrors::refusal_text(&error)),
                                NotifierType::Error,
                                NotifierIcon::None,
                                cx,
                            );
                            on_result(Err(error), cx);
                        }
                    });
                }),
                cx,
            );
        });
    }

    pub fn set_contact_photo(
        &mut self,
        target_user_id: &str,
        local_file: SaiWsOptimisticLocalFile,
        image_data: Option<Vec<u8>>,
        on_result: impl Fn(anyhow::Result<UserSavedContactWireDTO>, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        let on_result: UserContactResultHandler<UserSavedContactWireDTO> = Rc::new(on_result);
        let account_id = Self::contact_effective_user_id(cx);
        let pending = Self::contact_service(cx).read(cx).contact_photo_pending_targets.contains(target_user_id);
        tracing::warn!(target: "riseonly::X908", pending, has_account = account_id.is_some(), "contact photo start");
        let Some(account_id) = account_id.filter(|_| !pending) else {
            on_result(Err(UserContactMutationError::InvalidContact.into()), cx);
            return;
        };
        let target_user_id = target_user_id.to_owned();
        let request_id = Uuid::new_v4().uuid_string().to_lowercase();
        let upload = Self::contact_service(cx).update(cx, |service, cx| {
            service.upload_contact_photo(local_file, request_id.clone(), account_id.clone(), cx)
        });
        cx.spawn(async move |this, cx| {
            let upload = upload.await;
            tracing::warn!(target: "riseonly::X908", ok = upload.is_ok(), error = ?upload.as_ref().err(), "contact photo uploaded");
            let _ = this.update(cx, |store, cx| match upload {
                Ok(upload) => {
                    if !Self::is_active_contact_account(&account_id, cx) {
                        on_result(Err(CancellationError.into()), cx);
                        return;
                    }
                    store.set_uploaded_contact_photo_until_settled(
                        target_user_id,
                        upload,
                        request_id,
                        account_id,
                        image_data,
                        0,
                        on_result,
                        cx,
                    );
                }
                Err(error) if Self::is_contact_cancellation(&error) => on_result(Err(CancellationError.into()), cx),
                Err(error) => on_result(Err(error), cx),
            });
        })
        .detach();
    }

    #[allow(clippy::too_many_arguments)]
    fn set_uploaded_contact_photo_until_settled(
        &mut self,
        target_user_id: String,
        upload: UserContactPhotoUpload,
        client_request_id: String,
        account_id: String,
        image_data: Option<Vec<u8>>,
        attempt: i64,
        on_result: UserContactResultHandler<UserSavedContactWireDTO>,
        cx: &mut Context<Self>,
    ) {
        if !Self::is_active_contact_account(&account_id, cx) {
            on_result(Err(CancellationError.into()), cx);
            return;
        }
        let action_target = target_user_id.clone();
        let action_upload = upload.clone();
        let action_request_id = client_request_id.clone();
        Self::contact_actions(cx).update(cx, |actions, cx| {
            actions.set_saved_contact_photo_action(
                &action_target,
                Some(action_upload),
                Some(action_request_id.as_str()),
                Box::new(move |result, cx: &mut App| {
                    Self::with_contact_store(cx, move |_, cx| {
                        if !Self::is_active_contact_account(&account_id, cx) {
                            on_result(Err(CancellationError.into()), cx);
                            return;
                        }
                        match result {
                            Ok(contact) => {
                                let Some(image_data) = image_data else {
                                    on_result(Ok(contact), cx);
                                    return;
                                };
                                cx.spawn(async move |_, cx| {
                                    if !cx.update(|cx| Self::is_active_contact_account(&account_id, cx)) {
                                        cx.update(|cx| on_result(Err(CancellationError.into()), cx));
                                        return;
                                    }
                                    let book_account_id = account_id.clone();
                                    let book_target_user_id = target_user_id.clone();
                                    let updated = cx
                                        .background_executor()
                                        .spawn(async move {
                                            ContactDeviceBook::shared()
                                                .update_mapped_photo(&book_account_id, &book_target_user_id, image_data)
                                                .await
                                        })
                                        .await;
                                    cx.update(|cx| {
                                        if let Err(error) = updated {
                                            Sai::log(
                                                "User",
                                                move || format!("contact phone photo update failed: {error}"),
                                                SaiLogKind::Network,
                                                cx,
                                            );
                                        }
                                        on_result(Ok(contact), cx);
                                    });
                                })
                                .detach();
                            }
                            Err(error) => {
                                if attempt >= 5 || !Self::is_retryable_contact_photo_settlement_error(&error) {
                                    on_result(Err(error), cx);
                                    return;
                                }
                                let delay_milliseconds = 2_000_i64.min(350 * (attempt + 1));
                                let timer = cx
                                    .background_executor()
                                    .timer(Duration::from_millis(u64::try_from(delay_milliseconds).unwrap_or(0)));
                                cx.spawn(async move |_, cx| {
                                    timer.await;
                                    cx.update(|cx| {
                                        Self::with_contact_store(cx, move |store, cx| {
                                            store.set_uploaded_contact_photo_until_settled(
                                                target_user_id,
                                                upload,
                                                client_request_id,
                                                account_id,
                                                image_data,
                                                attempt + 1,
                                                on_result,
                                                cx,
                                            );
                                        });
                                    });
                                })
                                .detach();
                            }
                        }
                    });
                }),
                cx,
            );
        });
    }

    pub fn suggest_contact_photo(
        &mut self,
        target_user_id: &str,
        local_file: SaiWsOptimisticLocalFile,
        on_result: impl Fn(anyhow::Result<()>, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        let on_result: UserContactResultHandler<()> = Rc::new(on_result);
        let account_id = Self::contact_effective_user_id(cx);
        let pending =
            Self::contact_service(cx).read(cx).contact_photo_suggestion_pending_targets.contains(target_user_id);
        let Some(account_id) = account_id.filter(|_| !pending) else {
            on_result(Err(UserContactMutationError::InvalidContact.into()), cx);
            return;
        };
        let target_user_id = target_user_id.to_owned();
        let request_id = Uuid::new_v4().uuid_string().to_lowercase();
        let upload = Self::contact_service(cx).update(cx, |service, cx| {
            service.upload_contact_photo(local_file, request_id.clone(), account_id.clone(), cx)
        });
        cx.spawn(async move |this, cx| {
            let upload = upload.await;
            let _ = this.update(cx, |store, cx| match upload {
                Ok(upload) => {
                    if !Self::is_active_contact_account(&account_id, cx) {
                        on_result(Err(CancellationError.into()), cx);
                        return;
                    }
                    store.suggest_uploaded_contact_photo_until_settled(
                        target_user_id,
                        upload,
                        request_id,
                        account_id,
                        0,
                        on_result,
                        cx,
                    );
                }
                Err(error) if Self::is_contact_cancellation(&error) => on_result(Err(CancellationError.into()), cx),
                Err(error) => on_result(Err(error), cx),
            });
        })
        .detach();
    }

    #[allow(clippy::too_many_arguments)]
    fn suggest_uploaded_contact_photo_until_settled(
        &mut self,
        target_user_id: String,
        upload: UserContactPhotoUpload,
        client_request_id: String,
        account_id: String,
        attempt: i64,
        on_result: UserContactResultHandler<()>,
        cx: &mut Context<Self>,
    ) {
        if !Self::is_active_contact_account(&account_id, cx) {
            on_result(Err(CancellationError.into()), cx);
            return;
        }
        let action_target = target_user_id.clone();
        let action_upload = upload.clone();
        let action_request_id = client_request_id.clone();
        Self::contact_actions(cx).update(cx, |actions, cx| {
            actions.suggest_contact_photo_action(
                &action_target,
                action_upload,
                Some(action_request_id.as_str()),
                Box::new(move |result: anyhow::Result<UserContactPhotoSuggestionWireDTO>, cx: &mut App| {
                    Self::with_contact_store(cx, move |_, cx| {
                        if !Self::is_active_contact_account(&account_id, cx) {
                            on_result(Err(CancellationError.into()), cx);
                            return;
                        }
                        match result {
                            Ok(_) => {
                                Self::send_contact_notifier(
                                    tr("contact_photo_suggested"),
                                    None,
                                    NotifierType::Success,
                                    NotifierIcon::None,
                                    cx,
                                );
                                on_result(Ok(()), cx);
                            }
                            Err(error) => {
                                if attempt >= 5 || !Self::is_retryable_contact_photo_settlement_error(&error) {
                                    on_result(Err(error), cx);
                                    return;
                                }
                                let delay_milliseconds = 2_000_i64.min(350 * (attempt + 1));
                                let timer = cx
                                    .background_executor()
                                    .timer(Duration::from_millis(u64::try_from(delay_milliseconds).unwrap_or(0)));
                                cx.spawn(async move |_, cx| {
                                    timer.await;
                                    cx.update(|cx| {
                                        Self::with_contact_store(cx, move |store, cx| {
                                            store.suggest_uploaded_contact_photo_until_settled(
                                                target_user_id,
                                                upload,
                                                client_request_id,
                                                account_id,
                                                attempt + 1,
                                                on_result,
                                                cx,
                                            );
                                        });
                                    });
                                })
                                .detach();
                            }
                        }
                    });
                }),
                cx,
            );
        });
    }

    pub fn resolve_contact_photo_suggestion(
        &mut self,
        suggestion_id: &str,
        accept: bool,
        on_result: impl Fn(anyhow::Result<()>, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        let on_result: UserContactResultHandler<()> = Rc::new(on_result);
        let Some(account_id) = Self::contact_effective_user_id(cx) else {
            on_result(Err(UserContactMutationError::InvalidContact.into()), cx);
            return;
        };
        let resolution_key = format!("{suggestion_id}:{}", if accept { "accept" } else { "reject" });
        let request_id = Self::contact_service(cx).update(cx, |service, cx| {
            let request_id = service
                .contact_photo_resolution_request_ids
                .get(&resolution_key)
                .cloned()
                .unwrap_or_else(|| Uuid::new_v4().uuid_string().to_lowercase());
            service.contact_photo_resolution_request_ids.insert(resolution_key, request_id.clone());
            cx.notify();
            request_id
        });
        self.resolve_contact_photo_suggestion_until_settled(
            suggestion_id.to_owned(),
            accept,
            account_id,
            request_id,
            0,
            on_result,
            cx,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn resolve_contact_photo_suggestion_until_settled(
        &mut self,
        suggestion_id: String,
        accept: bool,
        account_id: String,
        client_request_id: String,
        attempt: i64,
        on_result: UserContactResultHandler<()>,
        cx: &mut Context<Self>,
    ) {
        if !Self::is_active_contact_account(&account_id, cx) {
            on_result(Err(CancellationError.into()), cx);
            return;
        }
        let action_suggestion_id = suggestion_id.clone();
        let action_request_id = client_request_id.clone();
        Self::contact_actions(cx).update(cx, |actions, cx| {
            actions.resolve_contact_photo_suggestion_action(
                &action_suggestion_id,
                accept,
                Some(action_request_id.as_str()),
                Box::new(move |result, cx: &mut App| {
                    Self::with_contact_store(cx, move |store, cx| {
                        if !Self::is_active_contact_account(&account_id, cx) {
                            on_result(Err(CancellationError.into()), cx);
                            return;
                        }
                        let resolution_key = format!("{suggestion_id}:{}", if accept { "accept" } else { "reject" });
                        match result {
                            Ok(suggestion) => {
                                let status = suggestion.status.to_uppercase();
                                let expected_final = if accept { "ACCEPTED" } else { "REJECTED" };
                                if status == expected_final {
                                    Self::contact_service(cx).update(cx, |service, cx| {
                                        service.contact_photo_resolution_request_ids.remove(&resolution_key);
                                        cx.notify();
                                    });
                                    if accept {
                                        store.apply_accepted_contact_photo_to_own_profile(&suggestion, &account_id, cx);
                                    }
                                    Self::contact_service(cx).update(cx, |service, cx| {
                                        service.finish_presenting_contact_photo_suggestion(cx);
                                    });
                                    on_result(Ok(()), cx);
                                    return;
                                }
                                on_result(Err(UserContactMutationError::InvalidResponse.into()), cx);
                            }
                            Err(error) => {
                                if Self::is_stale_contact_photo_suggestion_error(&error) {
                                    Self::contact_service(cx).update(cx, |service, cx| {
                                        service.contact_photo_resolution_request_ids.remove(&resolution_key);
                                        let _ = service
                                            .apply_contact_photo_suggestion_resolved_optimistic(&suggestion_id, cx);
                                        service.finish_presenting_contact_photo_suggestion(cx);
                                        cx.notify();
                                    });
                                    Self::send_contact_notifier(
                                        tr("contact_photo_suggestion_stale"),
                                        None,
                                        NotifierType::Warning,
                                        NotifierIcon::None,
                                        cx,
                                    );
                                    on_result(Ok(()), cx);
                                    return;
                                }
                                if attempt >= 5 || !Self::is_retryable_contact_photo_settlement_error(&error) {
                                    on_result(Err(error), cx);
                                    return;
                                }
                                store.schedule_contact_photo_resolution_retry(
                                    suggestion_id,
                                    accept,
                                    account_id,
                                    client_request_id,
                                    attempt,
                                    on_result,
                                    cx,
                                );
                            }
                        }
                    });
                }),
                cx,
            );
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn schedule_contact_photo_resolution_retry(
        &mut self,
        suggestion_id: String,
        accept: bool,
        account_id: String,
        client_request_id: String,
        attempt: i64,
        on_result: UserContactResultHandler<()>,
        cx: &mut Context<Self>,
    ) {
        let delay_milliseconds = 2_000_i64.min(350 * (attempt + 1));
        let timer =
            cx.background_executor().timer(Duration::from_millis(u64::try_from(delay_milliseconds).unwrap_or(0)));
        cx.spawn(async move |this, cx| {
            timer.await;
            let _ = this.update(cx, |store, cx| {
                store.resolve_contact_photo_suggestion_until_settled(
                    suggestion_id,
                    accept,
                    account_id,
                    client_request_id,
                    attempt + 1,
                    on_result,
                    cx,
                );
            });
        })
        .detach();
    }

    fn is_stale_contact_photo_suggestion_error(error: &anyhow::Error) -> bool {
        error.to_string().to_lowercase().contains("already resolved")
    }

    fn is_retryable_contact_photo_settlement_error(error: &anyhow::Error) -> bool {
        let value = error.to_string().to_lowercase();
        value.contains("unavailable")
            || value.contains("still being")
            || value.contains("temporar")
            || value.contains("not connected")
            || value.contains("connection")
            || value.contains("timed out")
    }

    fn apply_accepted_contact_photo_to_own_profile(
        &mut self,
        suggestion: &UserContactPhotoSuggestionWireDTO,
        account_id: &str,
        cx: &mut Context<Self>,
    ) {
        let avatar_url = trimming_whitespaces_and_newlines(&suggestion.stable_file_url).to_owned();
        if avatar_url.is_empty() {
            return;
        }
        let auth = AuthServicesStore::shared(cx);
        let Some(mut profile) = auth.read(cx).profile().cloned() else {
            return;
        };
        profile.avatar_url = Some(avatar_url.clone());
        profile.more.logo = avatar_url.clone();
        auth.update(cx, |auth, cx| auth.apply_updated_session_profile(profile, cx));
        UserProfilePhotosInteractionsStore::shared(cx).update(cx, |photos, cx| photos.invalidate_album(account_id, cx));

        let cache_key = SaiCache::new(["getMyProfile", account_id]).key;
        if SaiWsResponseCache::current_payload(
            &cache_key,
            SaiWsCachePreferredLayer::MemoryFirst,
            SaiWsDiskCachePolicy::PreserveExisting,
            cx,
        )
        .is_some()
        {
            let _ = SaiUpdater::update(
                &cache_key,
                |context| {
                    let mut root = AnyDictionaryBridge::dictionary(context.payload.as_ref()).unwrap_or_default();
                    if let Some(mut more) = AnyDictionaryBridge::dictionary(root.get(user_key::MORE)) {
                        more.insert(user_key::LOGO.to_owned(), Value::String(avatar_url.clone()));
                        root.insert(user_key::MORE.to_owned(), Value::Object(more));
                    }
                    root.insert(bot_key::AVATAR_URL.to_owned(), Value::String(avatar_url.clone()));
                    Ok(serde_json::to_vec(&Value::Object(root))?)
                },
                SaiUpdaterWriteScope::All,
                cx,
            );
        }
    }

    pub fn set_user_blocked(
        &mut self,
        target_user_id: &str,
        blocked: bool,
        on_result: impl Fn(anyhow::Result<bool>, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        self.set_user_block_scope(target_user_id, blocked.then_some(RiseBlockScope::Standard), on_result, cx);
    }

    pub fn set_user_block_scope(
        &mut self,
        target_user_id: &str,
        scope: Option<RiseBlockScope>,
        on_result: impl Fn(anyhow::Result<bool>, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        let target = trimming_whitespaces_and_newlines(target_user_id).to_owned();
        if target.is_empty()
            || Self::contact_effective_user_id(cx).as_deref() == Some(target.as_str())
            || Self::contact_service(cx).read(cx).contact_block_pending_targets.contains(&target)
        {
            return;
        }

        let previous = Self::contact_service(cx).update(cx, |service, cx| service.viewer_block_scope(&target, cx));
        if let Some(scope) = scope
            && previous == Some(scope)
        {
            return;
        }

        let toast_key = match (scope, previous) {
            (None, _) => "contact_unblocked_notification",
            (Some(_), None) => "contact_blocked_notification",
            (Some(_), Some(_)) => "contact_block_scope_changed",
        };

        if (user_guide_sandbox_hooks().owns_user_id)(&target, cx) {
            (user_guide_sandbox_hooks().set_panda_block_scope)(scope, cx);
            Self::send_contact_notifier(tr(toast_key), None, NotifierType::Success, NotifierIcon::None, cx);
            GuideSignalBus::emit(RiseGuideSignal::UserBlockSet);
            on_result(Ok(true), cx);
            return;
        }

        if !RiseTransportStatus::is_connected(cx) {
            RiseTransportStatus::wake_if_offline(cx);
            let error = anyhow::Error::new(SaiWsSendError::NotConnected);
            Self::send_contact_notifier(
                tr("contact_block_failed"),
                Some(CanonErrors::refusal_text(&error)),
                NotifierType::Error,
                NotifierIcon::None,
                cx,
            );
            on_result(Err(error), cx);
            return;
        }

        let action_target = target.clone();
        Self::contact_actions(cx).update(cx, |actions, cx| {
            actions.block_user_action_scope(
                &action_target,
                scope,
                Box::new(move |result, cx: &mut App| {
                    Self::with_contact_store(cx, move |store, cx| {
                        match &result {
                            Ok(_) => {
                                Self::send_contact_notifier(
                                    tr(toast_key),
                                    None,
                                    NotifierType::Success,
                                    NotifierIcon::None,
                                    cx,
                                );
                                GuideSignalBus::emit(RiseGuideSignal::UserBlockSet);
                                UserServicesHooks::refresh_listener_volumes(cx);
                                store.refresh_foreign_profile_user_id(target.clone(), cx);
                                let tag = Self::contact_service(cx)
                                    .update(cx, |service, cx| service.foreign_profile_tag(&target, cx));
                                if let Some(tag) = tag {
                                    store.refresh_foreign_profile_tag(tag, cx);
                                }
                                if scope.is_some() {
                                    store.refresh_my_profile(cx);
                                }
                            }
                            Err(error) => {
                                Self::send_contact_notifier(
                                    tr("contact_block_failed"),
                                    Some(CanonErrors::refusal_text(error)),
                                    NotifierType::Error,
                                    NotifierIcon::None,
                                    cx,
                                );
                            }
                        }
                        on_result(result, cx);
                    });
                }),
                cx,
            );
        });
    }

    pub fn refresh_block_state(&mut self, target_user_id: &str, cx: &mut Context<Self>) {
        let target = trimming_whitespaces_and_newlines(target_user_id).to_owned();
        if (user_guide_sandbox_hooks().owns_user_id)(&target, cx) {
            return;
        }
        if target.is_empty()
            || Self::contact_effective_user_id(cx).as_deref() == Some(target.as_str())
            || Self::contact_service(cx).read(cx).contact_block_pending_targets.contains(&target)
        {
            return;
        }

        match Self::contact_service(cx).update(cx, |service, cx| service.profile_block_fact(&target, cx)) {
            UserContactBlockFact::Resolved(scope) => {
                Self::contact_actions(cx).update(cx, |actions, cx| actions.note_block_scope_action(&target, scope, cx));
            }
            UserContactBlockFact::Unknown => self.load_foreign_profile_user_id(target, cx),
        }
    }

    pub fn load_blocked_users(&mut self, force: bool, cx: &mut Context<Self>) {
        Self::contact_actions(cx).update(cx, |actions, cx| actions.load_blocked_users_action(force, cx));
    }

    pub fn load_more_blocked_users(&mut self, cx: &mut Context<Self>) {
        let snapshot = Self::contact_service(cx).update(cx, |service, cx| service.blocked_users(cx));
        if !snapshot.has_more || snapshot.load_state != RiseUserBlockLoadState::Loaded {
            return;
        }

        Self::contact_actions(cx).update(cx, |actions, cx| actions.load_more_blocked_users_action(cx));
    }

    pub fn trim_blocked_users(&mut self, cx: &mut Context<Self>) {
        Self::contact_actions(cx).update(cx, |actions, cx| actions.trim_blocked_users_action(cx));
    }

    pub fn open_contact_join_virtual_chat(
        &mut self,
        chat: &ChatInfo,
        on_open: impl FnOnce(String, &mut App) + 'static,
        on_error: impl FnOnce(String, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        if !UserServicesStore::is_contact_join_virtual_chat_id(&chat.id) || chat.participant.id.is_empty() {
            on_error(tr("contact_join_chat_open_failed"), cx);
            return;
        }
        Self::list_contact_join_events(cx);
        let peer_id = chat.participant.id.clone();
        let participant = chat.participant.clone();
        ChatActionsStore::shared(cx).update(cx, |actions, cx| {
            actions.open_direct_chat_action(
                participant,
                Box::new(move |real_chat_id: String, cx: &mut App| {
                    Self::contact_service(cx).update(cx, |service, cx| {
                        service.migrate_contact_join_virtual_dialogs(&peer_id, &real_chat_id, cx);
                    });
                    on_open(real_chat_id, cx);
                }),
                Some(Box::new(on_error)),
                cx,
            );
        });
    }

    pub fn load_saved_contacts_for_chat_folders(&mut self, cx: &mut Context<Self>) {
        if Self::contact_actions(cx).read(cx).list_saved_contacts_sai.is_pending() {
            return;
        }
        Self::contact_service(cx).update(cx, |service, cx| {
            service.saved_contact_folder_bootstrap_complete = false;
            cx.notify();
        });
        Self::contact_actions(cx).update(cx, |actions, cx| {
            actions.list_saved_contacts_action(
                None,
                Some(Box::new(|success, cx: &mut App| {
                    Self::with_contact_store(cx, move |store, cx| {
                        if !success {
                            store.finish_saved_contact_folder_bootstrap(cx);
                            return;
                        }
                        let ids = store.saved_contact_folder_target_ids_including_visible_hydrated(cx);
                        ChatServicesStore::shared(cx).update(cx, |chat, cx| chat.apply_folder_contacts(ids, cx));
                        store.load_next_saved_contacts_page_for_chat_folders(1, cx);
                    });
                })),
                cx,
            );
        });
    }

    pub fn hydrate_visible_chat_participants_for_contact_folders(&mut self, cx: &mut Context<Self>) {
        if !Self::contact_service(cx).read(cx).saved_contact_folder_bootstrap_complete {
            return;
        }
        let Some(account_id) = Self::contact_effective_user_id(cx) else {
            return;
        };
        if !Self::is_active_contact_account(&account_id, cx) {
            return;
        }

        let ids = self.saved_contact_folder_target_ids_including_visible_hydrated(cx);
        ChatServicesStore::shared(cx).update(cx, |chat, cx| chat.apply_folder_contacts(ids, cx));
        let attempted_count =
            i64::try_from(Self::contact_service(cx).read(cx).saved_contact_folder_hydration_attempted_targets.len())
                .unwrap_or(i64::MAX);
        let available_budget =
            UserContactLimits::maximum_visible_saved_contact_hydrations().saturating_sub(attempted_count).max(0);
        if available_budget <= 0 {
            self.hydrate_next_visible_chat_participant_for_contact_folders(&account_id, cx);
            return;
        }

        let current_user_id = account_id.clone();
        let mut chats = ChatServicesStore::shared(cx).update(cx, |chat, cx| chat.current_chat_infos(cx));
        chats.sort_by(|lhs, rhs| rhs.updated_at.total_cmp(&lhs.updated_at));
        let service = Self::contact_service(cx);
        let candidates = service.update(cx, |service, cx| {
            let in_flight = service.saved_contact_folder_hydration_in_flight.clone();
            let queued: HashSet<String> = service.saved_contact_folder_hydration_queue.iter().cloned().collect();
            let mut seen = HashSet::new();
            let mut candidates = Vec::new();
            for chat in &chats {
                if i64::try_from(candidates.len()).unwrap_or(i64::MAX) >= available_budget {
                    break;
                }
                let participant_id = &chat.participant.id;
                if chat.r#type == ChatType::Direct
                    && !chat.participant.is_bot
                    && !participant_id.is_empty()
                    && *participant_id != current_user_id
                    && !UserServicesStore::is_contact_join_virtual_chat_id(&chat.id)
                    && !service.is_saved_contact_loaded(participant_id, cx)
                    && !service.saved_contact_folder_hydration_attempted_targets.contains(participant_id)
                    && !queued.contains(participant_id)
                    && in_flight.as_deref() != Some(participant_id.as_str())
                    && seen.insert(participant_id.clone())
                {
                    candidates.push(participant_id.clone());
                }
            }
            candidates
        });

        service.update(cx, |service, cx| {
            service.saved_contact_folder_hydration_queue.extend(candidates.iter().cloned());
            service.saved_contact_folder_hydration_attempted_targets.extend(candidates);
            cx.notify();
        });
        self.hydrate_next_visible_chat_participant_for_contact_folders(&account_id, cx);
    }

    fn ensure_contact_sync_settings_loaded(&mut self, cx: &mut Context<Self>) {
        let did_load = Self::contact_service(cx).read(cx).contact_sync_settings_did_load;
        let is_pending = Self::contact_actions(cx).read(cx).get_contact_sync_settings_sai.is_pending();
        if did_load || is_pending {
            if did_load {
                self.handle_loaded_contact_sync_settings(cx);
            }
            return;
        }
        Self::contact_actions(cx).update(cx, |actions, cx| {
            actions.get_contact_sync_settings_action(
                Some(Box::new(|result, cx: &mut App| {
                    if result.is_ok() {
                        Self::with_contact_store(cx, |store, cx| store.handle_loaded_contact_sync_settings(cx));
                    }
                })),
                cx,
            );
        });
    }

    fn handle_loaded_contact_sync_settings(&mut self, cx: &mut Context<Self>) {
        self.refresh_contact_authorization(cx);
        if !Self::contact_sync_enabled(cx) {
            if let Some(account_id) = Self::contact_effective_user_id(cx) {
                Self::cancel_contact_sync_run(cx);
                Self::contact_service(cx)
                    .update(cx, |service, cx| service.clear_contact_join_local_data(&account_id, cx));
                Self::clear_address_book_snapshot_in_background(account_id, cx);
            }
            return;
        }
        match Self::contact_authorization(cx) {
            ContactDeviceAuthorization::Authorized | ContactDeviceAuthorization::Limited => {
                self.schedule_address_book_sync(cx);
            }
            ContactDeviceAuthorization::Denied | ContactDeviceAuthorization::Restricted => {
                if contacts::contacts_support().is_supported() {
                    self.set_contact_sync_enabled(false, cx);
                }
            }
            ContactDeviceAuthorization::NotDetermined => {}
        }
    }

    fn update_contact_sync_preference(
        &mut self,
        enabled: bool,
        completion: Option<Box<dyn FnOnce(bool, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        Self::contact_actions(cx).update(cx, |actions, cx| {
            actions.update_contact_sync_settings_action(
                enabled,
                None,
                Some(Box::new(move |result, cx: &mut App| match result {
                    Ok(_) => {
                        if let Some(completion) = completion {
                            completion(true, cx);
                        }
                    }
                    Err(error) => {
                        Self::send_contact_notifier(
                            tr("contact_sync_update_failed"),
                            Some(CanonErrors::refusal_text(&error)),
                            NotifierType::Error,
                            NotifierIcon::None,
                            cx,
                        );
                        if let Some(completion) = completion {
                            completion(false, cx);
                        }
                    }
                })),
                cx,
            );
        });
    }

    pub fn sync_mapped_contact_phone(&mut self, target_user_id: &str, phone: Option<&str>, cx: &mut Context<Self>) {
        let normalized_phone = phone.map(trimming_whitespaces_and_newlines).unwrap_or_default().to_owned();
        if normalized_phone.is_empty() || target_user_id.is_empty() {
            return;
        }
        let Some(account_id) = Self::contact_effective_user_id(cx) else {
            return;
        };
        if !ContactDeviceBook::authorization().can_read_and_write() {
            return;
        }
        let target_user_id = target_user_id.to_owned();
        cx.spawn(async move |_, cx| {
            if !cx.update(|cx| Self::is_active_contact_account(&account_id, cx)) {
                return;
            }
            let added = cx
                .background_executor()
                .spawn(async move {
                    ContactDeviceBook::shared().add_mapped_phone(&account_id, &target_user_id, &normalized_phone).await
                })
                .await;
            if let Err(error) = added {
                cx.update(|cx| {
                    Sai::log(
                        "User",
                        move || format!("contact phone number update failed: {error}"),
                        SaiLogKind::Network,
                        cx,
                    );
                });
            }
        })
        .detach();
    }

    fn is_active_contact_account(account_id: &str, cx: &mut App) -> bool {
        let auth = AuthServicesStore::shared(cx);
        let auth = auth.read(cx);
        auth.is_authenticated() && auth.effective_user_id_for_wire().as_deref() == Some(account_id)
    }

    fn finish_contact_save_success(
        &mut self,
        contact: UserSavedContactWireDTO,
        on_result: &UserContactResultHandler<UserSavedContactWireDTO>,
        cx: &mut Context<Self>,
    ) {
        notification(NotificationKind::Success);
        Self::send_contact_notifier(
            tr("contact_saved_notification"),
            None,
            NotifierType::Success,
            NotifierIcon::System("person.crop.circle.badge.checkmark".to_owned()),
            cx,
        );
        on_result(Ok(contact), cx);
    }

    fn finish_contact_save_failure(
        &mut self,
        error: anyhow::Error,
        on_result: &UserContactResultHandler<UserSavedContactWireDTO>,
        cx: &mut Context<Self>,
    ) {
        notification(NotificationKind::Error);
        Self::send_contact_notifier(
            tr("contact_save_failed"),
            Some(CanonErrors::refusal_text(&error)),
            NotifierType::Error,
            NotifierIcon::None,
            cx,
        );
        on_result(Err(error), cx);
    }

    #[allow(clippy::too_many_arguments)]
    fn compensate_failed_contact_phone_save(
        &mut self,
        previous: Option<UserSavedContactWireDTO>,
        peer: UserContactPeer,
        target_user_id: String,
        account_id: String,
        original_error: anyhow::Error,
        on_result: UserContactResultHandler<UserSavedContactWireDTO>,
        cx: &mut Context<Self>,
    ) {
        if !Self::is_active_contact_account(&account_id, cx) {
            on_result(Err(CancellationError.into()), cx);
            return;
        }
        let finish = move |compensation: anyhow::Result<()>, cx: &mut App| {
            Self::with_contact_store(cx, move |store, cx| {
                if let Err(error) = compensation {
                    Sai::log(
                        "User",
                        move || format!("contact save compensation failed: {error}"),
                        SaiLogKind::Network,
                        cx,
                    );
                }
                store.finish_contact_save_failure(original_error, &on_result, cx);
            });
        };
        if let Some(previous) = previous {
            let restore_draft = UserContactDraft {
                target_user_id,
                first_name: previous.first_name,
                last_name: previous.last_name,
                note: previous.note,
                share_phone_number: previous.share_phone_number,
                sync_to_phone: previous.sync_to_phone,
            };
            Self::contact_actions(cx).update(cx, |actions, cx| {
                actions.upsert_saved_contact_action(
                    restore_draft,
                    peer,
                    None,
                    Box::new(move |result, cx: &mut App| finish(result.map(|_| ()), cx)),
                    cx,
                );
            });
        } else {
            Self::contact_actions(cx).update(cx, |actions, cx| {
                actions.delete_saved_contact_action(
                    &target_user_id,
                    None,
                    Box::new(finish),
                    cx,
                );
            });
        }
    }

    fn synchronize_address_book(&mut self, cx: &mut Context<Self>) {
        let Some(user_id) = Self::contact_effective_user_id(cx) else {
            return;
        };
        let did_load = Self::contact_service(cx).read(cx).contact_sync_settings_did_load;
        if !did_load || !Self::contact_sync_enabled(cx) || !Self::contact_authorization(cx).can_read_and_write() {
            return;
        }

        if Self::contact_service(cx).read(cx).contact_sync_in_progress {
            Self::contact_service(cx).update(cx, |service, _| service.contact_sync_rerun_requested = true);
            return;
        }

        let run_id = Uuid::new_v4();
        let previous_task = Self::contact_service(cx).update(cx, |service, _| service.contact_sync_task.take());
        drop(previous_task);
        Self::cancel_volatile_contact_sync_requests(cx);
        Self::contact_service(cx).update(cx, |service, cx| {
            service.contact_sync_run_id = Some(run_id);
            service.contact_sync_in_progress = true;
            service.contact_sync_rerun_requested = false;
            cx.notify();
        });
        let task = cx.spawn(async move |_, cx| {
            let _finish = UserContactSyncRunFinish { run_id, cx: cx.clone() };
            if !cx.update(|cx| Self::is_current_contact_sync_run(run_id, &user_id, cx)) {
                return;
            }
            cx.update(|cx| {
                Self::contact_service(cx).update(cx, |service, cx| {
                    service.contact_sync_retry_attempt = 0;
                    service.contact_sync_error = None;
                    cx.notify();
                });
            });
            let outcome = Self::run_address_book_sync(run_id, user_id.clone(), cx).await;
            let Err(error) = outcome else {
                return;
            };
            if Self::is_contact_cancellation(&error) {
                return;
            }
            cx.update(|cx| {
                if Self::is_current_contact_sync_run(run_id, &user_id, cx) {
                    let message = error.to_string();
                    Self::contact_service(cx).update(cx, |service, cx| {
                        service.contact_sync_error = Some(message.clone());
                        cx.notify();
                    });
                    let refusal = CanonErrors::failure(&error).map(|failure| failure.message).unwrap_or(message);
                    if refusal.contains("Contact synchronization is disabled") {
                        Self::with_contact_store(cx, |store, cx| store.reload_contact_sync_settings(cx));
                    }
                }
                Sai::log("User", move || format!("contact sync failed: {error}"), SaiLogKind::Network, cx);
            });
        });
        Self::contact_service(cx).update(cx, |service, _| service.contact_sync_task = Some(task));
    }

    async fn run_address_book_sync(run_id: Uuid, user_id: String, cx: &mut AsyncApp) -> anyhow::Result<()> {
        let book_user_id = user_id.clone();
        let plan = cx
            .background_executor()
            .spawn(async move { ContactDeviceBook::shared().prepare_sync(&book_user_id).await })
            .await?;
        if !cx.update(|cx| Self::is_current_contact_sync_run(run_id, &user_id, cx)) {
            return Err(CancellationError.into());
        }
        let last_completed = cx.update(|cx| Self::contact_service(cx).read(cx).contact_sync_last_completed_at.is_some());
        if plan.is_unchanged && last_completed {
            return Ok(());
        }
        let batch_size = cx.update(|cx| Self::contact_service(cx).update(cx, |service, cx| service.contact_sync_batch_size(cx)));
        let batches = Self::chunked(&plan.contacts, batch_size);
        let payload_batches = if batches.is_empty() { vec![Vec::new()] } else { batches };
        if i64::try_from(payload_batches.len()).unwrap_or(i64::MAX) > UserContactLimits::maximum_sync_batches() {
            return Err(UserContactMutationError::SyncLimitExceeded.into());
        }
        let total_batches = i64::try_from(payload_batches.len()).unwrap_or(i64::MAX);
        let sync_id = Uuid::new_v4().uuid_string();
        let client_request_ids: Vec<String> =
            payload_batches.iter().map(|_| Uuid::new_v4().uuid_string()).collect();
        let mut final_response: Option<SyncUserContactsGatewayDTO> = None;
        let mut attempt: i64 = 0;
        loop {
            final_response = None;
            let mut ladder: anyhow::Result<()> = Ok(());
            for (index, contacts) in payload_batches.iter().enumerate() {
                if !cx.update(|cx| Self::can_continue_contact_sync_run(run_id, &user_id, cx)) {
                    return Err(CancellationError.into());
                }
                let body = rise_request::user::SyncContacts {
                    client_request_id: client_request_ids[index].clone(),
                    device_id: plan.device_id.clone(),
                    sync_id: sync_id.clone(),
                    batch_index: Some(i64::try_from(index).unwrap_or(i64::MAX)),
                    contacts: Some(contacts.iter().map(|contact| contact.wire()).collect()),
                    device_generation: Some(plan.device_generation),
                    total_batches: Some(total_batches),
                };
                let request =
                    cx.update(|cx| Self::contact_actions(cx).update(cx, |actions, cx| actions.sync_contacts_action(body, cx)));
                match request.await {
                    Ok(response) => final_response = Some(response),
                    Err(error) => {
                        ladder = Err(error);
                        break;
                    }
                }
            }
            let Err(error) = ladder else {
                break;
            };
            if Self::is_contact_cancellation(&error) {
                return Err(CancellationError.into());
            }
            attempt += 1;
            let may_retry = attempt < UserContactLimits::MAXIMUM_SYNC_RETRY_ATTEMPTS
                && Self::is_retryable_contact_sync_error(&error)
                && cx.update(|cx| Self::can_continue_contact_sync_run(run_id, &user_id, cx));
            if !may_retry {
                return Err(error);
            }
            let delay = Self::contact_sync_retry_delay_nanoseconds(attempt);
            cx.update(|cx| {
                Self::contact_service(cx).update(cx, |service, cx| {
                    service.contact_sync_retry_attempt = attempt;
                    cx.notify();
                });
                Sai::log(
                    "User",
                    move || format!("contact sync temporary failure; retry {attempt}: {error}"),
                    SaiLogKind::Network,
                    cx,
                );
            });
            cx.background_executor().timer(Duration::from_nanos(delay)).await;
        }
        if !cx.update(|cx| Self::is_current_contact_sync_run(run_id, &user_id, cx)) {
            return Err(CancellationError.into());
        }
        let Some(final_response) = final_response.filter(|response| response.sync_completed) else {
            return Err(UserContactMutationError::InvalidResponse.into());
        };
        cx.background_executor().spawn(async move { ContactDeviceBook::shared().commit(&plan).await }).await?;
        cx.update(|cx| {
            Self::contact_service(cx).update(cx, |service, cx| {
                service.contact_sync_last_completed_at = Some(RiseClock::now());
                service.contact_sync_error = None;
                cx.notify();
            });
            let replaced = Self::contact_service(cx)
                .update(cx, |service, cx| service.replace_saved_contacts_from_completed_sync(&final_response, &user_id, cx));
            Self::with_contact_store(cx, move |store, cx| {
                if replaced {
                    store.finish_saved_contact_folder_bootstrap(cx);
                } else {
                    store.load_saved_contacts_for_chat_folders(cx);
                }
            });
        });
        Ok(())
    }

    fn finish_address_book_sync_run(&mut self, run_id: Uuid, cx: &mut Context<Self>) {
        let service = Self::contact_service(cx);
        let current = service.read(cx).contact_sync_run_id;
        if current != Some(run_id) && current.is_some() {
            return;
        }
        let rerun = service.update(cx, |service, cx| {
            service.contact_sync_in_progress = false;
            service.contact_sync_retry_attempt = 0;
            if let Some(task) = service.contact_sync_task.take() {
                task.detach();
            }
            service.contact_sync_run_id = None;
            let rerun = service.contact_sync_rerun_requested;
            service.contact_sync_rerun_requested = false;
            cx.notify();
            rerun
        });
        if rerun {
            self.schedule_address_book_sync(cx);
        }
    }

    fn is_current_contact_sync_run(run_id: Uuid, user_id: &str, cx: &mut App) -> bool {
        Self::contact_service(cx).read(cx).contact_sync_run_id == Some(run_id)
            && Self::is_active_contact_account(user_id, cx)
    }

    fn can_continue_contact_sync_run(run_id: Uuid, user_id: &str, cx: &mut App) -> bool {
        Self::is_current_contact_sync_run(run_id, user_id, cx)
            && Self::contact_sync_enabled(cx)
            && Self::contact_authorization(cx).can_read_and_write()
    }

    fn contact_sync_retry_delay_nanoseconds(attempt: i64) -> u64 {
        let exponent = (attempt - 1).clamp(0, 20);
        let base = UserContactLimits::INITIAL_SYNC_RETRY_DELAY_MILLISECONDS;
        let uncapped = base.checked_mul(1_i64 << exponent).unwrap_or(i64::MAX);
        let cap = uncapped.min(UserContactLimits::MAXIMUM_SYNC_RETRY_DELAY_MILLISECONDS);
        let jittered = cap as f64 * rand::random_range(0.5..=1.0);
        u64::try_from((jittered as i64).max(1)).unwrap_or(1) * 1_000_000
    }

    fn is_retryable_contact_sync_error(error: &anyhow::Error) -> bool {
        if Self::is_contact_cancellation(error) {
            return false;
        }
        if let Some(send_error) = error.downcast_ref::<SaiWsSendError>() {
            return match send_error {
                SaiWsSendError::NotConnected | SaiWsSendError::SendFailed(_) => true,
                SaiWsSendError::Maintenance | SaiWsSendError::EncodingFailed(_) | SaiWsSendError::GuideSandbox => {
                    false
                }
            };
        }
        if let Some(url_error) = error.downcast_ref::<URLError>() {
            return matches!(
                url_error.code,
                URLErrorCode::TimedOut
                    | URLErrorCode::CannotFindHost
                    | URLErrorCode::CannotConnectToHost
                    | URLErrorCode::NetworkConnectionLost
                    | URLErrorCode::DnsLookupFailed
                    | URLErrorCode::NotConnectedToInternet
                    | URLErrorCode::DataNotAllowed
            );
        }
        if error.downcast_ref::<UserContactMutationError>().is_some() {
            return false;
        }

        let value = error.to_string().to_lowercase();
        let terminal_markers =
            ["invalid_argument", "validation", "permission", "denied", "unauthenticated", "forbidden", "not authorized"];
        if terminal_markers.iter().any(|marker| value.contains(marker)) {
            return false;
        }
        let retryable_markers = [
            "unavailable",
            "temporar",
            "try again",
            "not connected",
            "connection",
            "transport",
            "timed out",
            "timeout",
            "network",
            "backfill",
            "warming",
        ];
        retryable_markers.iter().any(|marker| value.contains(marker))
    }

    fn schedule_address_book_sync(&mut self, cx: &mut Context<Self>) {
        SaiDebouncer::debounced_action_delay_milliseconds(
            "addressBook",
            Box::new(|cx: &mut App| Self::with_contact_store(cx, |store, cx| store.synchronize_address_book(cx))),
            1_500,
            "contactSync",
            1,
            None,
            cx,
        );
    }

    fn refresh_contact_authorization(&mut self, cx: &mut Context<Self>) {
        Self::set_contact_authorization(ContactDeviceBook::authorization(), cx);
    }

    fn install_contact_observers_if_needed(&mut self, cx: &mut Context<Self>) {
        let service = Self::contact_service(cx);
        if service.read(cx).contact_store_observer.is_none() {
            let (changed, mut on_changed) = mpsc::unbounded::<()>();
            if let Ok(watch) = contacts::watch_store_changes(move || {
                let _ = changed.unbounded_send(());
            }) {
                service.update(cx, |service, _| service.contact_store_observer = Some(watch));
                cx.spawn(async move |_, cx| {
                    while on_changed.next().await.is_some() {
                        cx.update(|cx| {
                            if !Self::contact_is_authenticated(cx) {
                                return;
                            }
                            Self::with_contact_store(cx, |store, cx| store.schedule_address_book_sync(cx));
                        });
                    }
                })
                .detach();
            }
        }
        if service.read(cx).contact_socket_observer.is_none() {
            let (ready, mut on_ready) = mpsc::unbounded::<()>();
            let subscription = RiseAppEvents::default_center().observe(RISE_ENGINE_CONNECTION_READY, move |_| {
                let _ = ready.unbounded_send(());
            });
            service.update(cx, |service, _| service.contact_socket_observer = Some(subscription));
            cx.spawn(async move |_, cx| {
                while on_ready.next().await.is_some() {
                    cx.update(|cx| {
                        Self::with_contact_store(cx, |store, cx| {
                            store.bootstrap_contacts_for_authenticated_account(cx);
                        });
                    });
                }
            })
            .detach();
        }
        if service.read(cx).contact_socket_drop_observer.is_none() {
            let (dropped, mut on_dropped) = mpsc::unbounded::<()>();
            let subscription = RiseAppEvents::default_center().observe(SAI_WS_DID_DISCONNECT, move |_| {
                let _ = dropped.unbounded_send(());
            });
            service.update(cx, |service, _| service.contact_socket_drop_observer = Some(subscription));
            cx.spawn(async move |_, cx| {
                while on_dropped.next().await.is_some() {
                    cx.update(Self::cancel_contact_sync_run);
                }
            })
            .detach();
        }
    }

    fn present_contact_permission_denied(&mut self, cx: &mut Context<Self>) {
        Self::send_contact_notifier(
            tr("contact_sync_permission_denied_title"),
            Some(tr("contact_sync_permission_denied_description")),
            NotifierType::Warning,
            NotifierIcon::System("person.crop.circle.badge.exclamationmark".to_owned()),
            cx,
        );
    }

    fn ensure_contact_write_authorization(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Task<anyhow::Result<ContactDeviceAuthorization>> {
        self.refresh_contact_authorization(cx);
        let current = Self::contact_authorization(cx);
        if current != ContactDeviceAuthorization::NotDetermined {
            return Task::ready(Ok(current));
        }
        cx.spawn(async move |_, cx| {
            let authorization = ContactDeviceBook::shared().request_authorization().await?;
            cx.update(|cx| Self::set_contact_authorization(authorization, cx));
            Ok(authorization)
        })
    }

    fn load_next_saved_contacts_page_for_chat_folders(&mut self, automatic_pages_loaded: i64, cx: &mut Context<Self>) {
        let account_id = Self::contact_effective_user_id(cx).unwrap_or_default();
        let (saved_count, has_more, relative_id) = Self::contact_service(cx).update(cx, |service, cx| {
            (service.saved_contacts(cx).len(), service.saved_contacts_has_more(cx), service.saved_contacts_relative_id(cx))
        });
        let list_key = UserServicesStore::saved_contacts_list_cache_key(&account_id);
        let can_request = SaiWs::pagination_state(&list_key, cx).read(cx).trailing().can_request_automatically();
        let relative_id = relative_id.filter(|_| {
            automatic_pages_loaded < UserContactLimits::maximum_automatic_saved_contact_pages()
                && i64::try_from(saved_count).unwrap_or(i64::MAX)
                    < UserContactLimits::maximum_automatic_saved_contact_ids()
                && has_more
                && can_request
        });
        let Some(relative_id) = relative_id else {
            self.finish_saved_contact_folder_bootstrap(cx);
            return;
        };
        Self::contact_actions(cx).update(cx, |actions, cx| {
            actions.list_saved_contacts_action(
                Some(relative_id.as_str()),
                Some(Box::new(move |success, cx: &mut App| {
                    Self::with_contact_store(cx, move |store, cx| {
                        if !success {
                            store.finish_saved_contact_folder_bootstrap(cx);
                            return;
                        }
                        let ids = store.saved_contact_folder_target_ids_including_visible_hydrated(cx);
                        ChatServicesStore::shared(cx).update(cx, |chat, cx| chat.apply_folder_contacts(ids, cx));
                        store.load_next_saved_contacts_page_for_chat_folders(automatic_pages_loaded + 1, cx);
                    });
                })),
                cx,
            );
        });
    }

    fn finish_saved_contact_folder_bootstrap(&mut self, cx: &mut Context<Self>) {
        Self::contact_service(cx).update(cx, |service, cx| {
            service.saved_contact_folder_bootstrap_complete = true;
            cx.notify();
        });
        let ids = self.saved_contact_folder_target_ids_including_visible_hydrated(cx);
        ChatServicesStore::shared(cx).update(cx, |chat, cx| chat.apply_folder_contacts(ids, cx));
        self.hydrate_visible_chat_participants_for_contact_folders(cx);
    }

    fn hydrate_next_visible_chat_participant_for_contact_folders(&mut self, account_id: &str, cx: &mut Context<Self>) {
        if !Self::is_active_contact_account(account_id, cx) {
            return;
        }
        let service = Self::contact_service(cx);
        if !service.read(cx).saved_contact_folder_bootstrap_complete
            || service.read(cx).saved_contact_folder_hydration_in_flight.is_some()
        {
            return;
        }

        loop {
            let next = service.update(cx, |service, cx| {
                if service.saved_contact_folder_hydration_queue.is_empty() {
                    return None;
                }
                let target_user_id = service.saved_contact_folder_hydration_queue.remove(0);
                cx.notify();
                Some(target_user_id)
            });
            let Some(target_user_id) = next else {
                return;
            };
            if service.update(cx, |service, cx| service.is_saved_contact_loaded(&target_user_id, cx)) {
                continue;
            }

            service.update(cx, |service, cx| {
                service.saved_contact_folder_hydration_in_flight = Some(target_user_id.clone());
                cx.notify();
            });
            let account_id = account_id.to_owned();
            let action_target = target_user_id.clone();
            Self::contact_actions(cx).update(cx, |actions, cx| {
                actions.get_saved_contact_action(
                    &action_target,
                    Some(Box::new(move |_, cx: &mut App| {
                        Self::with_contact_store(cx, move |store, cx| {
                            if !Self::is_active_contact_account(&account_id, cx) {
                                return;
                            }
                            let service = Self::contact_service(cx);
                            if service.read(cx).saved_contact_folder_hydration_in_flight.as_deref()
                                != Some(target_user_id.as_str())
                            {
                                return;
                            }
                            service.update(cx, |service, cx| {
                                service.saved_contact_folder_hydration_in_flight = None;
                                cx.notify();
                            });
                            let ids = store.saved_contact_folder_target_ids_including_visible_hydrated(cx);
                            ChatServicesStore::shared(cx).update(cx, |chat, cx| chat.apply_folder_contacts(ids, cx));
                            store.hydrate_next_visible_chat_participant_for_contact_folders(&account_id, cx);
                        });
                    })),
                    cx,
                );
            });
            return;
        }
    }

    fn saved_contact_folder_target_ids_including_visible_hydrated(&mut self, cx: &mut Context<Self>) -> Vec<String> {
        let chats = ChatServicesStore::shared(cx).update(cx, |chat, cx| chat.current_chat_infos(cx));
        Self::contact_service(cx).update(cx, |service, cx| {
            let mut ids = service.saved_contact_target_ids(cx);
            for chat in chats.iter().filter(|chat| chat.r#type == ChatType::Direct && !chat.participant.id.is_empty()) {
                if service.saved_contact_gateway(&chat.participant.id, cx).is_some_and(|gateway| gateway.is_contact) {
                    ids.insert(chat.participant.id.clone());
                }
            }
            ids.into_iter().collect()
        })
    }

    fn chunked<Element: Clone>(elements: &[Element], size: i64) -> Vec<Vec<Element>> {
        let Ok(size) = usize::try_from(size) else {
            return Vec::new();
        };
        if size == 0 || elements.is_empty() {
            return Vec::new();
        }
        elements.chunks(size).map(<[Element]>::to_vec).collect()
    }

    fn contact_service(cx: &mut App) -> Entity<UserServicesStore> {
        UserServicesStore::shared(cx)
    }

    fn contact_actions(cx: &mut App) -> Entity<UserActionsStore> {
        UserActionsStore::shared(cx)
    }

    fn contact_is_authenticated(cx: &mut App) -> bool {
        AuthServicesStore::shared(cx).read(cx).is_authenticated()
    }

    fn contact_effective_user_id(cx: &mut App) -> Option<String> {
        AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire()
    }

    fn contact_authorization(cx: &mut App) -> ContactDeviceAuthorization {
        Self::contact_service(cx).read(cx).contact_authorization
    }

    fn set_contact_authorization(authorization: ContactDeviceAuthorization, cx: &mut App) {
        Self::contact_service(cx).update(cx, |service, cx| {
            service.contact_authorization = authorization;
            cx.notify();
        });
    }

    fn contact_sync_enabled(cx: &mut App) -> bool {
        Self::contact_service(cx).update(cx, |service, cx| service.contact_sync_enabled(cx))
    }

    fn claim_contact_foreground_refresh(kind: UserServicesStoreContactForegroundRefresh, cx: &mut App) -> bool {
        Self::contact_service(cx).update(cx, |service, cx| service.claim_contact_foreground_refresh(kind, RiseClock::now(), cx))
    }

    fn list_contact_join_events(cx: &mut App) {
        Self::contact_actions(cx).update(cx, |actions, cx| actions.list_contact_join_events_action(None, 0, cx));
    }

    fn cancel_volatile_contact_sync_requests(cx: &mut App) {
        SaiWsDeferredOutboundCoordinator::shared(cx).update(cx, |coordinator, cx| {
            coordinator.cancel_volatile_requests(
                rise_wire_methods::user::SERVICE,
                rise_wire_methods::user::SYNC_CONTACTS,
                CancellationError.into(),
                cx,
            );
        });
    }

    fn cancel_contact_sync_run(cx: &mut App) {
        let previous_task = Self::contact_service(cx).update(cx, |service, _| service.contact_sync_task.take());
        drop(previous_task);
        Self::cancel_volatile_contact_sync_requests(cx);
        Self::contact_service(cx).update(cx, |service, cx| {
            service.contact_sync_run_id = None;
            service.contact_sync_in_progress = false;
            service.contact_sync_retry_attempt = 0;
            cx.notify();
        });
    }

    fn clear_address_book_snapshot_in_background(account_id: String, cx: &mut App) {
        cx.background_executor()
            .spawn(async move {
                let _ = ContactDeviceBook::shared().clear_address_book_snapshot(&account_id).await;
            })
            .detach();
    }

    fn is_contact_cancellation(error: &anyhow::Error) -> bool {
        error.downcast_ref::<CancellationError>().is_some()
    }

    fn send_contact_notifier(
        title: String,
        content: Option<String>,
        r#type: NotifierType,
        icon: NotifierIcon,
        cx: &mut App,
    ) {
        Notifier::send(
            title,
            NotifierSendOptions {
                content,
                r#type,
                position: NotifierPosition::Bottom,
                icon,
                ..NotifierSendOptions::default()
            },
            cx,
        );
    }

    fn with_contact_store(cx: &mut App, update: impl FnOnce(&mut Self, &mut Context<Self>) + 'static) {
        cx.defer(move |cx| {
            Self::shared(cx).update(cx, update);
        });
    }
}
