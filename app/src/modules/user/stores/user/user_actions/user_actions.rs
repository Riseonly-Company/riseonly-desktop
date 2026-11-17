use std::any::Any;
use std::rc::Rc;

use gpui::{App, Context, Entity, Global};
use rise_canon::rise_key::user as key;
use rise_canon::rise_request::user::update_privacy_settings;
use rise_canon::{rise_request, rise_response, rise_wire_methods};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::core::lib::canon_errors::CanonErrors;
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::core::utilities::sai_cache::SaiCache;
use crate::core::lib::sai::core::utilities::sai_cache_cleaner::SaiCacheCleanLayer;
use crate::core::lib::sai::core::utilities::sai_payload_path::SaiJSONCodec;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::{SaiTypedSlice, SaiWireEmptyDTO};
use crate::core::lib::sai::modules::sai_ws::sai_ws::{SaiWs, SaiWsOnError, SaiWsOnSuccess};
use crate::core::lib::sai::modules::sai_ws::sai_ws_pagination::{
    SaiWsPaginationDuplicatePolicy, SaiWsPaginationOptions, SaiWsPaginationRelativeIdArgs,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{
    SaiWsCachePreferredLayer, SaiWsRequestOptions, SaiWsRequestOptionsInit, SaiWsSendError,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws_response_cache::SaiWsResponseCache;
use crate::core::lib::sai::sai::Sai;
use crate::modules::auth::services::access_token_user_id::AccessTokenUserId;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::stores::chats::chat_interactions::chat_interactions::ChatInteractionsStore;
use crate::modules::user::shared::profile_missing_tag_state::ProfileMissingTagState;
use crate::modules::user::stores::user::user_interactions::user_interactions::UserInteractionsStore;
use crate::modules::user::stores::user::user_services::user_guide_sandbox_hooks::user_guide_sandbox_hooks;
use crate::modules::user::stores::user::user_services::user_services::UserServicesStore;

use super::user_contact_wire_types::{
    MarkUserContactJoinEventsReadGatewayDTO, SyncUserContactsGatewayDTO, UserContactJoinEventsGatewayDTO,
    UserContactPhotoSuggestionGatewayDTO, UserContactPhotoSuggestionsListGatewayDTO,
    UserContactSyncSettingsGatewayDTO, UserSavedContactGatewayDTO, UserSavedContactsListGatewayDTO,
};
use super::user_types::{PrivacyRulePatch, UserListKind, UserListRelationship};
use super::user_wire_types::{
    ChatMuteOverridesGatewayDTO, FriendListGatewayDTO, FriendRequestGatewayDTO, PrivacySettingsWireDTO,
    RiseUserProfileRow, SyncProfileGoalsGatewayDTO, SyncProfilePlansGatewayDTO, UnfollowUserGatewayDTO,
    UpdateProfileMoreBody, UserGetProfileByTagGatewayDTO, UserGetProfileByTagGatewayDTOExt,
    UserProfileSubscriptionDTO, UserSubsListGatewayDTO,
};

pub struct UpdateProfileActionArgs {
    pub name: Option<String>,
    pub tag: Option<String>,
    pub phone: Option<String>,
    pub gender: Option<String>,
    pub theme_id: Option<String>,
    pub quick_reaction: Option<String>,
    pub premium_badge_emoji: Option<String>,
    pub name_color: Option<String>,
    pub more: Option<UpdateProfileMoreBody>,
    pub allows_deferred_send: bool,
    pub on_success: Option<Rc<dyn Fn(&mut App)>>,
    pub on_error: Option<Rc<dyn Fn(&anyhow::Error, &mut App)>>,
}

impl Default for UpdateProfileActionArgs {
    fn default() -> Self {
        Self {
            name: None,
            tag: None,
            phone: None,
            gender: None,
            theme_id: None,
            quick_reaction: None,
            premium_badge_emoji: None,
            name_color: None,
            more: None,
            allows_deferred_send: true,
            on_success: None,
            on_error: None,
        }
    }
}

pub struct UserActionsStore {
    pub my_profile_sai: Rc<SaiTypedSlice<UserProfileSubscriptionDTO>>,
    pub get_profile_by_tag_sai: Rc<SaiTypedSlice<UserGetProfileByTagGatewayDTO>>,
    pub get_profile_by_id_sai: Rc<SaiTypedSlice<UserGetProfileByTagGatewayDTO>>,
    pub update_profile_sai: Rc<SaiTypedSlice<UserProfileSubscriptionDTO>>,
    pub set_phone_public_sai: Rc<SaiTypedSlice<rise_response::user::SetPhonePublic>>,
    pub get_privacy_sai: Rc<SaiTypedSlice<PrivacySettingsWireDTO>>,
    pub update_privacy_sai: Rc<SaiTypedSlice<PrivacySettingsWireDTO>>,
    pub privacy_exception_sai: Rc<SaiTypedSlice<SaiWireEmptyDTO>>,
    pub get_user_friends_sai: Rc<SaiTypedSlice<FriendListGatewayDTO>>,
    pub get_user_subs_sai: Rc<SaiTypedSlice<UserSubsListGatewayDTO>>,
    pub get_user_subscriptions_sai: Rc<SaiTypedSlice<UserSubsListGatewayDTO>>,
    pub send_friend_request_sai: Rc<SaiTypedSlice<FriendRequestGatewayDTO>>,
    pub cancel_friend_request_sai: Rc<SaiTypedSlice<SaiWireEmptyDTO>>,
    pub accept_friend_request_sai: Rc<SaiTypedSlice<SaiWireEmptyDTO>>,
    pub reject_friend_request_sai: Rc<SaiTypedSlice<SaiWireEmptyDTO>>,
    pub delete_friend_sai: Rc<SaiTypedSlice<SaiWireEmptyDTO>>,
    pub unfollow_user_sai: Rc<SaiTypedSlice<UnfollowUserGatewayDTO>>,
    pub update_wallet_sai: Rc<SaiTypedSlice<SaiWireEmptyDTO>>,
    pub sync_profile_goals_sai: Rc<SaiTypedSlice<SyncProfileGoalsGatewayDTO>>,
    pub sync_profile_plans_sai: Rc<SaiTypedSlice<SyncProfilePlansGatewayDTO>>,
    pub set_mute_all_chats_sai: Rc<SaiTypedSlice<SaiWireEmptyDTO>>,
    pub get_chat_mute_overrides_sai: Rc<SaiTypedSlice<ChatMuteOverridesGatewayDTO>>,
    pub add_chat_mute_override_sai: Rc<SaiTypedSlice<SaiWireEmptyDTO>>,
    pub remove_chat_mute_override_sai: Rc<SaiTypedSlice<SaiWireEmptyDTO>>,
    pub get_contact_sync_settings_sai: Rc<SaiTypedSlice<UserContactSyncSettingsGatewayDTO>>,
    pub update_contact_sync_settings_sai: Rc<SaiTypedSlice<UserContactSyncSettingsGatewayDTO>>,
    pub sync_contacts_sai: Rc<SaiTypedSlice<SyncUserContactsGatewayDTO>>,
    pub get_saved_contact_sai: Rc<SaiTypedSlice<UserSavedContactGatewayDTO>>,
    pub list_saved_contacts_sai: Rc<SaiTypedSlice<UserSavedContactsListGatewayDTO>>,
    pub upsert_saved_contact_sai: Rc<SaiTypedSlice<UserSavedContactGatewayDTO>>,
    pub delete_saved_contact_sai: Rc<SaiTypedSlice<UserSavedContactGatewayDTO>>,
    pub set_saved_contact_photo_sai: Rc<SaiTypedSlice<UserSavedContactGatewayDTO>>,
    pub suggest_contact_photo_sai: Rc<SaiTypedSlice<UserContactPhotoSuggestionGatewayDTO>>,
    pub list_contact_photo_suggestions_sai: Rc<SaiTypedSlice<UserContactPhotoSuggestionsListGatewayDTO>>,
    pub resolve_contact_photo_suggestion_sai: Rc<SaiTypedSlice<UserContactPhotoSuggestionGatewayDTO>>,
    pub list_contact_join_events_sai: Rc<SaiTypedSlice<UserContactJoinEventsGatewayDTO>>,
    pub mark_contact_join_events_read_sai: Rc<SaiTypedSlice<MarkUserContactJoinEventsReadGatewayDTO>>,
}

/// shared handles of the slices services read, store keeps the same Rc so both see one state
pub struct UserActionsSlices {
    pub get_profile_by_tag_sai: Rc<SaiTypedSlice<UserGetProfileByTagGatewayDTO>>,
    pub get_profile_by_id_sai: Rc<SaiTypedSlice<UserGetProfileByTagGatewayDTO>>,
    pub get_privacy_sai: Rc<SaiTypedSlice<PrivacySettingsWireDTO>>,
    pub get_user_friends_sai: Rc<SaiTypedSlice<FriendListGatewayDTO>>,
    pub get_user_subs_sai: Rc<SaiTypedSlice<UserSubsListGatewayDTO>>,
    pub get_user_subscriptions_sai: Rc<SaiTypedSlice<UserSubsListGatewayDTO>>,
    pub get_saved_contact_sai: Rc<SaiTypedSlice<UserSavedContactGatewayDTO>>,
}

impl UserActionsSlices {
    fn of(store: &UserActionsStore) -> Self {
        Self {
            get_profile_by_tag_sai: store.get_profile_by_tag_sai.clone(),
            get_profile_by_id_sai: store.get_profile_by_id_sai.clone(),
            get_privacy_sai: store.get_privacy_sai.clone(),
            get_user_friends_sai: store.get_user_friends_sai.clone(),
            get_user_subs_sai: store.get_user_subs_sai.clone(),
            get_user_subscriptions_sai: store.get_user_subscriptions_sai.clone(),
            get_saved_contact_sai: store.get_saved_contact_sai.clone(),
        }
    }
}

struct UserActionsSliceHandles(Rc<UserActionsSlices>);

impl Global for UserActionsSliceHandles {}

impl UserActionsStore {
    pub const DEFAULT_LIST_LIMIT: i64 = 20;

    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(cx: &mut Context<Self>) -> Self {
        let store = Self {
            my_profile_sai: SaiTypedSlice::new(cx),
            get_profile_by_tag_sai: SaiTypedSlice::new(cx),
            get_profile_by_id_sai: SaiTypedSlice::new(cx),
            update_profile_sai: SaiTypedSlice::new(cx),
            set_phone_public_sai: SaiTypedSlice::new(cx),
            get_privacy_sai: SaiTypedSlice::new(cx),
            update_privacy_sai: SaiTypedSlice::new(cx),
            privacy_exception_sai: SaiTypedSlice::new(cx),
            get_user_friends_sai: SaiTypedSlice::new(cx),
            get_user_subs_sai: SaiTypedSlice::new(cx),
            get_user_subscriptions_sai: SaiTypedSlice::new(cx),
            send_friend_request_sai: SaiTypedSlice::new(cx),
            cancel_friend_request_sai: SaiTypedSlice::new(cx),
            accept_friend_request_sai: SaiTypedSlice::new(cx),
            reject_friend_request_sai: SaiTypedSlice::new(cx),
            delete_friend_sai: SaiTypedSlice::new(cx),
            unfollow_user_sai: SaiTypedSlice::new(cx),
            update_wallet_sai: SaiTypedSlice::new(cx),
            sync_profile_goals_sai: SaiTypedSlice::new(cx),
            sync_profile_plans_sai: SaiTypedSlice::new(cx),
            set_mute_all_chats_sai: SaiTypedSlice::new(cx),
            get_chat_mute_overrides_sai: SaiTypedSlice::new(cx),
            add_chat_mute_override_sai: SaiTypedSlice::new(cx),
            remove_chat_mute_override_sai: SaiTypedSlice::new(cx),
            get_contact_sync_settings_sai: SaiTypedSlice::new(cx),
            update_contact_sync_settings_sai: SaiTypedSlice::new(cx),
            sync_contacts_sai: SaiTypedSlice::new(cx),
            get_saved_contact_sai: SaiTypedSlice::new(cx),
            list_saved_contacts_sai: SaiTypedSlice::new(cx),
            upsert_saved_contact_sai: SaiTypedSlice::new(cx),
            delete_saved_contact_sai: SaiTypedSlice::new(cx),
            set_saved_contact_photo_sai: SaiTypedSlice::new(cx),
            suggest_contact_photo_sai: SaiTypedSlice::new(cx),
            list_contact_photo_suggestions_sai: SaiTypedSlice::new(cx),
            resolve_contact_photo_suggestion_sai: SaiTypedSlice::new(cx),
            list_contact_join_events_sai: SaiTypedSlice::new(cx),
            mark_contact_join_events_read_sai: SaiTypedSlice::new(cx),
        };
        cx.set_global(UserActionsSliceHandles(Rc::new(UserActionsSlices::of(&store))));
        store
    }

    /// slices services read without leasing this store,
    /// a read inside an update of the store would abort the app
    pub fn slices(cx: &mut App) -> Rc<UserActionsSlices> {
        if let Some(handles) = cx.try_global::<UserActionsSliceHandles>() {
            return handles.0.clone();
        }
        let actions = Self::shared(cx);
        Rc::new(UserActionsSlices::of(actions.read(cx)))
    }

    fn active_profile_id(cx: &mut App) -> Option<String> {
        AuthServicesStore::shared(cx)
            .read(cx)
            .profile()
            .map(|profile| profile.id.clone())
            .filter(|user_id| !user_id.is_empty())
    }

    fn is_authenticated(cx: &mut App) -> bool {
        AuthServicesStore::shared(cx).read(cx).is_authenticated()
    }

    fn profile_or_token_user_id(cx: &mut App) -> Option<String> {
        let auth = AuthServicesStore::shared(cx).read(cx);
        auth.profile()
            .map(|profile| profile.id.clone())
            .or_else(|| AccessTokenUserId::extract(&auth.tokens_and_other_data().access_token))
    }

    fn log_network(text: String, cx: &mut App) {
        Sai::log("User", move || text, SaiLogKind::Network, cx);
    }

    fn decoded_payload<T: DeserializeOwned + Clone + 'static>(payload: Option<&dyn Any>) -> Option<T> {
        let payload = payload?;
        if let Some(decoded) = payload.downcast_ref::<T>() {
            return Some(decoded.clone());
        }
        SaiJSONCodec::decode::<T>(payload.downcast_ref::<Value>())
    }

    fn mutation_options(method: &str) -> SaiWsRequestOptions {
        SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::user::SERVICE.to_owned(),
            method: method.to_owned(),
            reflect_pending_in_ui: false,
            store_response_in_cache: false,
            ..SaiWsRequestOptionsInit::default()
        })
    }

    fn foreign_profile_options(method: &str, cache_key: &str) -> SaiWsRequestOptions {
        SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::user::SERVICE.to_owned(),
            method: method.to_owned(),
            cache_key: Some(cache_key.to_owned()),
            reload_if_cached: true,
            allow_concurrent_while_pending: false,
            reflect_pending_in_ui: true,
            send_immediately: true,
            store_response_in_cache: true,
            persist_cache_to_disk: false,
            reload_when_only_disk_cached: true,
            preferred_cache_layer: SaiWsCachePreferredLayer::MemoryFirst,
            ..SaiWsRequestOptionsInit::default()
        })
    }

    fn list_options(
        method: &str,
        cache_key: &str,
        cursor: Option<&str>,
        relative_id: Option<i64>,
        is_first_page: bool,
    ) -> SaiWsRequestOptions {
        SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::user::SERVICE.to_owned(),
            method: method.to_owned(),
            cache_key: Some(cache_key.to_owned()),
            reload_if_cached: true,
            allow_concurrent_while_pending: false,
            reflect_pending_in_ui: is_first_page,
            send_immediately: true,
            store_response_in_cache: true,
            persist_cache_to_disk: false,
            reload_when_only_disk_cached: true,
            preferred_cache_layer: SaiWsCachePreferredLayer::MemoryFirst,
            path_to_array: Some(key::ITEMS.to_owned()),
            path_to_id: Some(key::ENTRY_KEY.to_owned()),
            pagination: Some(SaiWsPaginationOptions::relative_id(SaiWsPaginationRelativeIdArgs {
                relative_id: cursor.map(str::to_owned).or_else(|| relative_id.map(|id| id.to_string())),
                response_relative_id_path: key::CURSOR.to_owned(),
                response_relative_id_fallback_path: Some(key::RELATIVE_ID.to_owned()),
                has_more_path: Some(key::HAS_MORE.to_owned()),
                duplicate_policy: SaiWsPaginationDuplicatePolicy::UseIncoming,
                ..SaiWsPaginationRelativeIdArgs::default()
            })),
            ..SaiWsRequestOptionsInit::default()
        })
    }

    fn apply_foreign_profile_fetch(
        response: &UserGetProfileByTagGatewayDTO,
        user_id: &str,
        cache_key: &str,
        cx: &mut App,
    ) {
        UserServicesStore::shared(cx).update(cx, |services, cx| {
            services.note_foreign_profile_fetched(cache_key, cx);
            services.note_peer_restriction(user_id, response, cx);
            services.clear_follow_overlay_on_fetch(user_id, cx);
            services.reconcile_contact_metadata_from_foreign_profile(response, user_id, cx);
        });
        if let Some(profile) = response.as_profile_model() {
            ChatInteractionsStore::shared(cx).update(cx, |chat, cx| {
                chat.apply_peer_logo(user_id, &profile.more.logo, cx);
            });
            UserInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                interactions.sync_mapped_contact_phone(user_id, profile.more.phone.as_deref(), cx);
            });
        }
    }
}

impl UserActionsStore {
    pub fn get_my_profile_action(&mut self, cx: &mut Context<Self>) {
        if !Self::is_authenticated(cx) {
            return;
        }
        let Some(user_id) = Self::profile_or_token_user_id(cx).filter(|user_id| !user_id.is_empty()) else {
            return;
        };

        let cache_key = SaiCache::new(["getMyProfile", user_id.as_str()]).key;
        let body = rise_request::user::GetMyProfile {};
        let phone_public_revision = UserServicesStore::shared(cx).read(cx).phone_public_mutation_revision();
        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::user::SERVICE.to_owned(),
            method: rise_wire_methods::user::GET_MY_PROFILE.to_owned(),
            cache_key: Some(cache_key),
            reload_if_cached: true,
            allow_concurrent_while_pending: false,
            reflect_pending_in_ui: false,
            send_immediately: true,
            store_response_in_cache: true,
            persist_cache_to_disk: true,
            reload_when_only_disk_cached: true,
            preferred_cache_layer: SaiWsCachePreferredLayer::DiskFirst,
            take_path: Some(key::PROFILE.to_owned()),
            ..SaiWsRequestOptionsInit::default()
        });

        let slice = Rc::clone(&self.my_profile_sai);
        let success: SaiWsOnSuccess = Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| {
            let profile = slice.data();
            UserServicesStore::shared(cx).update(cx, |services, cx| {
                services.handle_my_profile_success(profile, phone_public_revision, cx);
            });
        });
        let failure: SaiWsOnError = Rc::new(|error: &anyhow::Error, cx: &mut App| {
            tracing::warn!(target: "riseonly::X453", "my_profile error: {error}");
            Self::log_network(format!("get_my_profile error: {error}"), cx);
        });
        SaiWs::request_slice(&self.my_profile_sai, body, options, Some(success), Some(failure), cx);
    }

    pub fn get_profile_by_tag_action(
        &mut self,
        tag: &str,
        on_success: Option<SaiWsOnSuccess>,
        on_error: Option<SaiWsOnError>,
        cx: &mut Context<Self>,
    ) {
        let tag = trimming_whitespaces_and_newlines(tag).to_owned();
        if tag.is_empty() {
            return;
        }

        let body = rise_request::user::GetProfileByTag::new(tag.clone());
        let cache_key = UserServicesStore::profile_by_tag_cache_key(&tag);
        let options = Self::foreign_profile_options(rise_wire_methods::user::GET_PROFILE_BY_TAG, &cache_key);
        let account_user_id = options.account_user_id.clone();
        let account_epoch = options.account_epoch.clone();

        let missing_tag_request = ProfileMissingTagState::shared(cx).update(cx, |state, cx| {
            state.begin(&tag, account_user_id.as_deref(), account_epoch.as_deref(), cx)
        });

        let success_request = missing_tag_request.clone();
        let success_account_user_id = account_user_id.clone();
        let success_account_epoch = account_epoch.clone();
        let success: SaiWsOnSuccess = Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
            if SaiAccountCacheScope::is_current_account_epoch(
                success_account_user_id.as_deref(),
                success_account_epoch.as_deref(),
            ) {
                ProfileMissingTagState::shared(cx).update(cx, |state, cx| {
                    state.finish(success_request.as_ref(), None, cx);
                });
            }
            if let Some(response) = Self::decoded_payload::<UserGetProfileByTagGatewayDTO>(payload.as_deref()) {
                let user_id = UserServicesStore::shared(cx).update(cx, |services, cx| {
                    services.validated_foreign_profile_user_id(&response, None, Some(tag.as_str()), cx)
                });
                if let Some(user_id) = user_id {
                    Self::apply_foreign_profile_fetch(&response, &user_id, &cache_key, cx);
                }
            }
            if let Some(on_success) = &on_success {
                on_success(payload, cx);
            }
        });
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            if SaiAccountCacheScope::is_current_account_epoch(account_user_id.as_deref(), account_epoch.as_deref()) {
                let failure = CanonErrors::failure(error);
                ProfileMissingTagState::shared(cx).update(cx, |state, cx| {
                    state.finish(missing_tag_request.as_ref(), failure.as_ref(), cx);
                });
            }
            Self::log_network(format!("get_profile_by_tag error: {error}"), cx);
            if let Some(on_error) = &on_error {
                on_error(error, cx);
            }
        });
        SaiWs::request_slice(&self.get_profile_by_tag_sai, body, options, Some(success), Some(failure), cx);
    }

    pub fn get_profile_by_id_action(
        &mut self,
        user_id: &str,
        on_success: Option<SaiWsOnSuccess>,
        on_error: Option<SaiWsOnError>,
        cx: &mut Context<Self>,
    ) {
        let profile_user_id = trimming_whitespaces_and_newlines(user_id).to_owned();
        if profile_user_id.is_empty() {
            return;
        }

        let raw_viewer_id = Self::profile_or_token_user_id(cx);
        let viewer_id = raw_viewer_id
            .as_deref()
            .map(trimming_whitespaces_and_newlines)
            .unwrap_or_default()
            .to_owned();
        if viewer_id.is_empty() {
            return;
        }

        let body = rise_request::user::GetProfileById::new(profile_user_id.clone());
        let cache_key = UserServicesStore::profile_by_id_cache_key(&profile_user_id, &viewer_id);
        let options = Self::foreign_profile_options(rise_wire_methods::user::GET_PROFILE_BY_ID, &cache_key);
        let account_user_id = options.account_user_id.clone();
        let account_epoch = options.account_epoch.clone();

        let missing_id_request = ProfileMissingTagState::shared(cx).update(cx, |state, cx| {
            let key = ProfileMissingTagState::user_id_key(&profile_user_id);
            state.begin(&key, account_user_id.as_deref(), account_epoch.as_deref(), cx)
        });

        let success_request = missing_id_request.clone();
        let success_account_user_id = account_user_id.clone();
        let success_account_epoch = account_epoch.clone();
        let success: SaiWsOnSuccess = Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
            if SaiAccountCacheScope::is_current_account_epoch(
                success_account_user_id.as_deref(),
                success_account_epoch.as_deref(),
            ) {
                ProfileMissingTagState::shared(cx).update(cx, |state, cx| {
                    state.finish(success_request.as_ref(), None, cx);
                });
            }
            if let Some(response) = Self::decoded_payload::<UserGetProfileByTagGatewayDTO>(payload.as_deref()) {
                let user_id = UserServicesStore::shared(cx).update(cx, |services, cx| {
                    services.validated_foreign_profile_user_id(&response, Some(profile_user_id.as_str()), None, cx)
                });
                if let Some(user_id) = user_id {
                    Self::apply_foreign_profile_fetch(&response, &user_id, &cache_key, cx);
                }
            }
            if let Some(on_success) = &on_success {
                on_success(payload, cx);
            }
        });
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            if SaiAccountCacheScope::is_current_account_epoch(account_user_id.as_deref(), account_epoch.as_deref()) {
                let failure = CanonErrors::failure(error);
                ProfileMissingTagState::shared(cx).update(cx, |state, cx| {
                    state.finish(missing_id_request.as_ref(), failure.as_ref(), cx);
                });
            }
            Self::log_network(format!("get_profile_by_id error: {error}"), cx);
            if let Some(on_error) = &on_error {
                on_error(error, cx);
            }
        });
        SaiWs::request_slice(&self.get_profile_by_id_sai, body, options, Some(success), Some(failure), cx);
    }

    pub fn update_profile_action(&mut self, args: UpdateProfileActionArgs, cx: &mut Context<Self>) {
        let UpdateProfileActionArgs {
            name,
            tag,
            phone,
            gender,
            theme_id,
            quick_reaction,
            premium_badge_emoji,
            name_color,
            more,
            allows_deferred_send,
            on_success,
            on_error,
        } = args;
        let Some(user_id) = Self::active_profile_id(cx) else {
            if let Some(on_error) = &on_error {
                on_error(&anyhow::Error::new(SaiWsSendError::SendFailed("No active profile".to_owned())), cx);
            }
            return;
        };

        let body = rise_request::user::UpdateProfile {
            gender,
            more,
            name,
            name_color,
            phone,
            premium_badge_emoji,
            quick_reaction,
            tag,
            theme_id,
        };
        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::user::SERVICE.to_owned(),
            method: rise_wire_methods::user::UPDATE_PROFILE.to_owned(),
            reflect_pending_in_ui: false,
            store_response_in_cache: false,
            allows_deferred_send,
            ..SaiWsRequestOptionsInit::default()
        });

        let slice = Rc::clone(&self.update_profile_sai);
        let success: SaiWsOnSuccess = Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| {
            if let Some(profile) = slice.data() {
                let model = profile.as_profile_model();
                AuthServicesStore::shared(cx).update(cx, |auth, cx| auth.apply_updated_session_profile(model, cx));
            }
            SaiWsResponseCache::remove(
                &SaiCache::new(["getMyProfile", user_id.as_str()]).key,
                SaiCacheCleanLayer::Everywhere,
                cx,
            );
            if let Some(on_success) = &on_success {
                on_success(cx);
            }
        });
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            Self::log_network(format!("update_profile error: {error}"), cx);
            if let Some(on_error) = &on_error {
                on_error(error, cx);
            }
        });
        SaiWs::request_slice(&self.update_profile_sai, body, options, Some(success), Some(failure), cx);
    }

    pub fn set_phone_public_action(&mut self, is_phone_public: bool, cx: &mut Context<Self>) {
        if Self::active_profile_id(cx).is_none() {
            return;
        }

        UserServicesStore::shared(cx).update(cx, |services, cx| {
            services.apply_phone_public_optimistic(is_phone_public, cx);
        });

        let body = rise_request::user::SetPhonePublic::new(is_phone_public);
        let options = Self::mutation_options(rise_wire_methods::user::SET_PHONE_PUBLIC);

        let slice = Rc::clone(&self.set_phone_public_sai);
        let success: SaiWsOnSuccess = Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| {
            let profile = slice.data().and_then(|data| data.profile);
            UserServicesStore::shared(cx).update(cx, |services, cx| {
                services.commit_phone_public_optimistic(profile, cx);
            });
        });
        let failure: SaiWsOnError = Rc::new(|error: &anyhow::Error, cx: &mut App| {
            UserServicesStore::shared(cx).update(cx, |services, cx| services.revert_phone_public_optimistic(cx));
            Self::log_network(format!("set_phone_public error: {error}"), cx);
        });
        SaiWs::request_slice(&self.set_phone_public_sai, body, options, Some(success), Some(failure), cx);
    }

    pub fn get_privacy_action(&mut self, cx: &mut Context<Self>) {
        let Some(user_id) = Self::active_profile_id(cx) else {
            return;
        };

        let body = rise_request::user::GetPrivacySettings {};
        let cache_key = SaiCache::new(["getPrivacySettings", user_id.as_str()]).key;
        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::user::SERVICE.to_owned(),
            method: rise_wire_methods::user::GET_PRIVACY_SETTINGS.to_owned(),
            cache_key: Some(cache_key),
            reload_if_cached: true,
            allow_concurrent_while_pending: false,
            reflect_pending_in_ui: false,
            send_immediately: true,
            store_response_in_cache: true,
            persist_cache_to_disk: true,
            reload_when_only_disk_cached: true,
            preferred_cache_layer: SaiWsCachePreferredLayer::DiskFirst,
            take_path: Some(key::SETTINGS.to_owned()),
            ..SaiWsRequestOptionsInit::default()
        });

        SaiWs::request_slice(&self.get_privacy_sai, body, options, None, None, cx);
    }
}

impl UserActionsStore {
    fn privacy_rule<Rule: DeserializeOwned>(raw: Option<&str>) -> Option<Rule> {
        raw.and_then(|raw| serde_json::from_value::<Rule>(Value::String(raw.to_owned())).ok())
    }

    pub fn update_privacy_action(
        &mut self,
        patch: PrivacyRulePatch,
        on_error: Option<Rc<dyn Fn(&anyhow::Error, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        if Self::active_profile_id(cx).is_none() {
            return;
        }

        let server = self.get_privacy_sai.data();
        UserServicesStore::shared(cx)
            .update(cx, |services, cx| services.apply_privacy_patch_optimistic(&patch, server, cx));

        let body = rise_request::user::UpdatePrivacySettings {
            achievements_rule: Self::privacy_rule::<update_privacy_settings::AchievementsRule>(
                patch.achievements_rule.as_deref(),
            ),
            calls_rule: Self::privacy_rule::<update_privacy_settings::CallsRule>(patch.calls_rule.as_deref()),
            coauthor_rule: Self::privacy_rule::<update_privacy_settings::CoauthorRule>(patch.coauthor_rule.as_deref()),
            comments_rule: Self::privacy_rule::<update_privacy_settings::CommentsRule>(patch.comments_rule.as_deref()),
            description_rule: Self::privacy_rule::<update_privacy_settings::DescriptionRule>(
                patch.description_rule.as_deref(),
            ),
            forwards_rule: Self::privacy_rule::<update_privacy_settings::ForwardsRule>(patch.forwards_rule.as_deref()),
            friend_rule: Self::privacy_rule::<update_privacy_settings::FriendRule>(patch.friend_rule.as_deref()),
            gift_send_rule: Self::privacy_rule::<update_privacy_settings::GiftSendRule>(
                patch.gift_send_rule.as_deref(),
            ),
            gifts_rule: Self::privacy_rule::<update_privacy_settings::GiftsRule>(patch.gifts_rule.as_deref()),
            goal_rule: Self::privacy_rule::<update_privacy_settings::GoalRule>(patch.goal_rule.as_deref()),
            groups_rule: Self::privacy_rule::<update_privacy_settings::GroupsRule>(patch.groups_rule.as_deref()),
            hb_rule: Self::privacy_rule::<update_privacy_settings::HbRule>(patch.hb_rule.as_deref()),
            last_seen_rule: Self::privacy_rule::<update_privacy_settings::LastSeenRule>(
                patch.last_seen_rule.as_deref(),
            ),
            message_requests_enabled: patch.message_requests_enabled,
            messages_rule: Self::privacy_rule::<update_privacy_settings::MessagesRule>(patch.messages_rule.as_deref()),
            online_status_rule: Self::privacy_rule::<update_privacy_settings::OnlineStatusRule>(
                patch.online_status_rule.as_deref(),
            ),
            phone_rule: Self::privacy_rule::<update_privacy_settings::PhoneRule>(patch.phone_rule.as_deref()),
            plan_rule: Self::privacy_rule::<update_privacy_settings::PlanRule>(patch.plan_rule.as_deref()),
            profile_photo_rule: Self::privacy_rule::<update_privacy_settings::ProfilePhotoRule>(
                patch.profile_photo_rule.as_deref(),
            ),
            reposts_rule: Self::privacy_rule::<update_privacy_settings::RepostsRule>(patch.reposts_rule.as_deref()),
            sparks_rule: Self::privacy_rule::<update_privacy_settings::SparksRule>(patch.sparks_rule.as_deref()),
            stories_rule: Self::privacy_rule::<update_privacy_settings::StoriesRule>(patch.stories_rule.as_deref()),
            subs_count_hidden: patch.subs_count_hidden,
            subs_rule: Self::privacy_rule::<update_privacy_settings::SubsRule>(patch.subs_rule.as_deref()),
            tag_rule: Self::privacy_rule::<update_privacy_settings::TagRule>(patch.tag_rule.as_deref()),
        };
        let options = Self::mutation_options(rise_wire_methods::user::UPDATE_PRIVACY_SETTINGS);

        let slice = Rc::clone(&self.update_privacy_sai);
        let success: SaiWsOnSuccess = Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| {
            let settings = slice.data();
            UserServicesStore::shared(cx).update(cx, |services, cx| {
                services.handle_update_privacy_success(settings, cx);
            });
        });
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            UserServicesStore::shared(cx).update(cx, |services, cx| services.rollback_privacy_optimistic(cx));
            Self::log_network(format!("update_privacy_settings error: {error}"), cx);
            if let Some(on_error) = &on_error {
                on_error(error, cx);
            }
        });
        SaiWs::request_slice(&self.update_privacy_sai, body, options, Some(success), Some(failure), cx);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn set_privacy_exception_action(
        &mut self,
        target_user_id: &str,
        privacy_type: &str,
        action: &str,
        remove: bool,
        on_success: Rc<dyn Fn(&mut App)>,
        on_error: Rc<dyn Fn(&anyhow::Error, &mut App)>,
        cx: &mut Context<Self>,
    ) {
        if Self::active_profile_id(cx).is_none() {
            return;
        }

        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::user::SERVICE.to_owned(),
            method: if remove {
                rise_wire_methods::user::DELETE_PRIVACY_EXCEPTION.to_owned()
            } else {
                rise_wire_methods::user::ADD_PRIVACY_EXCEPTION.to_owned()
            },
            allow_concurrent_while_pending: true,
            reflect_pending_in_ui: false,
            store_response_in_cache: false,
            ..SaiWsRequestOptionsInit::default()
        });
        let on_done: SaiWsOnSuccess = Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| on_success(cx));
        let on_fail: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            Self::log_network(format!("privacy exception error: {error}"), cx);
            on_error(error, cx);
        });

        if remove {
            SaiWs::request_slice(
                &self.privacy_exception_sai,
                rise_request::user::DeletePrivacyException::new(
                    action.to_owned(),
                    privacy_type.to_owned(),
                    target_user_id.to_owned(),
                ),
                options,
                Some(on_done),
                Some(on_fail),
                cx,
            );
        } else {
            SaiWs::request_slice(
                &self.privacy_exception_sai,
                rise_request::user::AddPrivacyException::new(
                    action.to_owned(),
                    privacy_type.to_owned(),
                    target_user_id.to_owned(),
                ),
                options,
                Some(on_done),
                Some(on_fail),
                cx,
            );
        }
    }
}

impl UserActionsStore {
    fn relationship_list_callbacks(
        kind: UserListKind,
        user_id: &str,
        search: Option<&str>,
        cache_key: &str,
        error_label: &'static str,
        on_success: Option<Rc<dyn Fn(&mut App)>>,
        on_error: Option<Rc<dyn Fn(&anyhow::Error, &mut App)>>,
    ) -> (SaiWsOnSuccess, SaiWsOnError) {
        let success_user_id = user_id.to_owned();
        let success_search = search.map(str::to_owned);
        let success_cache_key = cache_key.to_owned();
        let success: SaiWsOnSuccess = Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| {
            UserServicesStore::shared(cx).update(cx, |services, cx| {
                let _ = services.register_list_cache_scope(kind, &success_user_id, success_search.as_deref(), cx);
                services.clear_list_failure(&success_cache_key, cx);
            });
            if let Some(on_success) = &on_success {
                on_success(cx);
            }
        });
        let failure_user_id = user_id.to_owned();
        let failure_search = search.map(str::to_owned);
        let failure_cache_key = cache_key.to_owned();
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            UserServicesStore::shared(cx).update(cx, |services, cx| {
                let _ = services.register_list_cache_scope(kind, &failure_user_id, failure_search.as_deref(), cx);
                services.mark_list_failure(&failure_cache_key, cx);
            });
            if let Some(on_error) = &on_error {
                on_error(error, cx);
            }
            Self::log_network(format!("{error_label} error: {error}"), cx);
        });
        (success, failure)
    }

    fn register_relationship_list(
        kind: UserListKind,
        user_id: &str,
        search: Option<&str>,
        cx: &mut App,
    ) -> String {
        UserServicesStore::shared(cx).update(cx, |services, cx| {
            let cache_key = services.register_list_cache_scope(kind, user_id, search, cx);
            services.clear_list_failure(&cache_key, cx);
            cache_key
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn get_user_friends_action(
        &mut self,
        user_id: &str,
        cursor: Option<&str>,
        relative_id: Option<i64>,
        limit: i64,
        search: Option<&str>,
        on_success: Option<Rc<dyn Fn(&mut App)>>,
        on_error: Option<Rc<dyn Fn(&anyhow::Error, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        if user_id.is_empty() {
            return;
        }

        let q = UserServicesStore::normalized_list_query(search);
        let is_searching = !q.as_deref().unwrap_or_default().is_empty();
        let body = rise_request::user::GetUserFriends {
            cursor: cursor.map(str::to_owned),
            limit: Some(limit),
            relative_id,
            search: if is_searching { q.clone() } else { None },
            up: Some(cursor.is_some() || relative_id.is_some()),
            user_id: user_id.to_owned(),
        };
        let is_first_page = cursor.is_none() && relative_id.is_none();
        let cache_key = Self::register_relationship_list(UserListKind::Friends, user_id, q.as_deref(), cx);
        let options = Self::list_options(
            rise_wire_methods::user::GET_USER_FRIENDS,
            &cache_key,
            cursor,
            relative_id,
            is_first_page,
        );
        let (success, failure) = Self::relationship_list_callbacks(
            UserListKind::Friends,
            user_id,
            q.as_deref(),
            &cache_key,
            "get_user_friends",
            on_success,
            on_error,
        );
        SaiWs::request_slice(&self.get_user_friends_sai, body, options, Some(success), Some(failure), cx);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn get_user_subscribers_action(
        &mut self,
        user_id: &str,
        cursor: Option<&str>,
        relative_id: Option<i64>,
        limit: i64,
        search: Option<&str>,
        on_success: Option<Rc<dyn Fn(&mut App)>>,
        on_error: Option<Rc<dyn Fn(&anyhow::Error, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        if user_id.is_empty() {
            return;
        }

        let q = UserServicesStore::normalized_list_query(search);
        let is_searching = !q.as_deref().unwrap_or_default().is_empty();
        let body = rise_request::user::GetUserSubscriptions {
            cursor: cursor.map(str::to_owned),
            limit: Some(limit),
            relative_id,
            search: if is_searching { q.clone() } else { None },
            up: Some(cursor.is_some() || relative_id.is_some()),
            user_id: user_id.to_owned(),
        };
        let is_first_page = cursor.is_none() && relative_id.is_none();
        let cache_key = Self::register_relationship_list(UserListKind::Subscribers, user_id, q.as_deref(), cx);
        let options = Self::list_options(
            rise_wire_methods::user::GET_USER_SUBSCRIPTIONS,
            &cache_key,
            cursor,
            relative_id,
            is_first_page,
        );
        let (success, failure) = Self::relationship_list_callbacks(
            UserListKind::Subscribers,
            user_id,
            q.as_deref(),
            &cache_key,
            "get_user_subscriptions",
            on_success,
            on_error,
        );
        SaiWs::request_slice(&self.get_user_subs_sai, body, options, Some(success), Some(failure), cx);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn get_user_subscriptions_action(
        &mut self,
        user_id: &str,
        cursor: Option<&str>,
        relative_id: Option<i64>,
        limit: i64,
        search: Option<&str>,
        on_success: Option<Rc<dyn Fn(&mut App)>>,
        on_error: Option<Rc<dyn Fn(&anyhow::Error, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        if user_id.is_empty() {
            return;
        }

        let q = UserServicesStore::normalized_list_query(search);
        let is_searching = !q.as_deref().unwrap_or_default().is_empty();
        let body = rise_request::user::GetUserSubs {
            cursor: cursor.map(str::to_owned),
            limit: Some(limit),
            relative_id,
            search: if is_searching { q.clone() } else { None },
            up: Some(cursor.is_some() || relative_id.is_some()),
            user_id: user_id.to_owned(),
        };
        let is_first_page = cursor.is_none() && relative_id.is_none();
        let cache_key = Self::register_relationship_list(UserListKind::Subscriptions, user_id, q.as_deref(), cx);
        let options =
            Self::list_options(rise_wire_methods::user::GET_USER_SUBS, &cache_key, cursor, relative_id, is_first_page);
        let (success, failure) = Self::relationship_list_callbacks(
            UserListKind::Subscriptions,
            user_id,
            q.as_deref(),
            &cache_key,
            "get_user_subs",
            on_success,
            on_error,
        );
        SaiWs::request_slice(&self.get_user_subscriptions_sai, body, options, Some(success), Some(failure), cx);
    }
}

impl UserActionsStore {
    pub fn send_friend_request_action(
        &mut self,
        receiver_id: &str,
        profile_tag: Option<&str>,
        comment: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        if Self::active_profile_id(cx).is_none() {
            return;
        }

        let body = rise_request::user::SendFriendRequest {
            comment: comment.map(str::to_owned),
            receiver_id: receiver_id.to_owned(),
        };
        let options = Self::mutation_options(rise_wire_methods::user::SEND_FRIEND_REQUEST);

        let slice = Rc::clone(&self.send_friend_request_sai);
        let profile_tag = profile_tag.map(str::to_owned);
        let success: SaiWsOnSuccess = Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| {
            let Some(request_id) = slice.data().map(|request| request.id) else {
                return;
            };
            let Some(tag) = profile_tag.as_deref().filter(|tag| !tag.is_empty()) else {
                return;
            };
            UserServicesStore::shared(cx).update(cx, |services, cx| {
                services.patch_profile_by_tag_friend_request_id(Some(request_id), tag, cx);
            });
        });
        SaiWs::request_slice(&self.send_friend_request_sai, body, options, Some(success), None, cx);
    }

    pub fn cancel_friend_request_action(&mut self, request_id: i64, profile_tag: Option<&str>, cx: &mut Context<Self>) {
        if Self::active_profile_id(cx).is_none() {
            return;
        }

        let body = rise_request::user::CancelFriendRequest::new(request_id);
        let options = Self::mutation_options(rise_wire_methods::user::CANCEL_FRIEND_REQUEST);

        let profile_tag = profile_tag.map(str::to_owned);
        let success: SaiWsOnSuccess = Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| {
            let Some(tag) = profile_tag.as_deref().filter(|tag| !tag.is_empty()) else {
                return;
            };
            UserServicesStore::shared(cx).update(cx, |services, cx| {
                services.patch_profile_by_tag_friend_request_id(None, tag, cx);
            });
        });
        SaiWs::request_slice(&self.cancel_friend_request_sai, body, options, Some(success), None, cx);
    }

    pub fn accept_friend_request_action(&mut self, request_id: i64, cx: &mut Context<Self>) {
        if Self::active_profile_id(cx).is_none() {
            return;
        }

        let body = rise_request::user::AcceptFriendRequest::new(request_id);
        let options = Self::mutation_options(rise_wire_methods::user::ACCEPT_FRIEND_REQUEST);
        SaiWs::request_slice(&self.accept_friend_request_sai, body, options, None, None, cx);
    }

    pub fn reject_friend_request_action(&mut self, request_id: i64, cx: &mut Context<Self>) {
        if Self::active_profile_id(cx).is_none() {
            return;
        }

        let body = rise_request::user::RejectFriendRequest::new(request_id);
        let options = Self::mutation_options(rise_wire_methods::user::REJECT_FRIEND_REQUEST);
        SaiWs::request_slice(&self.reject_friend_request_sai, body, options, None, None, cx);
    }

    pub fn delete_friend_action(&mut self, friend_id: &str, cx: &mut Context<Self>) {
        if Self::active_profile_id(cx).is_none() {
            return;
        }

        let body = rise_request::user::DeleteFriend::new(friend_id.to_owned());
        let options = Self::mutation_options(rise_wire_methods::user::DELETE_FRIEND);
        SaiWs::request_slice(&self.delete_friend_sai, body, options, None, None, cx);
    }

    pub fn follow_user_action(
        &mut self,
        target_id: &str,
        on_success: Option<Rc<dyn Fn(Option<UserListRelationship>, &mut App)>>,
        on_error: Option<Rc<dyn Fn(&anyhow::Error, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        if Self::active_profile_id(cx).is_none() {
            return;
        }

        let body = rise_request::user::SendFriendRequest { comment: None, receiver_id: target_id.to_owned() };
        let options = Self::mutation_options(rise_wire_methods::user::SEND_FRIEND_REQUEST);

        let success: SaiWsOnSuccess = Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| {
            if let Some(on_success) = &on_success {
                on_success(None, cx);
            }
        });
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            if let Some(on_error) = &on_error {
                on_error(error, cx);
            }
        });
        SaiWs::request_slice(&self.send_friend_request_sai, body, options, Some(success), Some(failure), cx);
    }

    pub fn unfollow_user_action(
        &mut self,
        target_id: &str,
        on_success: Option<Rc<dyn Fn(Option<UserListRelationship>, &mut App)>>,
        on_error: Option<Rc<dyn Fn(&anyhow::Error, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        if Self::active_profile_id(cx).is_none() {
            return;
        }

        let body = rise_request::user::UnfollowUser::new(target_id.to_owned());
        let options = Self::mutation_options(rise_wire_methods::user::UNFOLLOW_USER);

        let slice = Rc::clone(&self.unfollow_user_sai);
        let success: SaiWsOnSuccess = Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
            let response =
                Self::decoded_payload::<UnfollowUserGatewayDTO>(payload.as_deref()).or_else(|| slice.data());
            let relationship = response.map(|response| UserListRelationship {
                is_subbed: response.is_subbed,
                is_subscriber: response.is_subscriber,
                is_friend: response.is_friend,
            });
            if let Some(on_success) = &on_success {
                on_success(relationship, cx);
            }
        });
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            if let Some(on_error) = &on_error {
                on_error(error, cx);
            }
        });
        SaiWs::request_slice(&self.unfollow_user_sai, body, options, Some(success), Some(failure), cx);
    }

    pub fn update_wallet_settings_action(
        &mut self,
        wallet_enabled: Option<bool>,
        chain_account_ss58: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        let Some(user_id) = Self::active_profile_id(cx) else {
            return;
        };

        let body = rise_request::user::UpdateWalletSettings {
            chain_account_ss58: chain_account_ss58.map(str::to_owned),
            user_id: Some(user_id),
            wallet_enabled,
        };
        let options = Self::mutation_options(rise_wire_methods::user::UPDATE_WALLET_SETTINGS);

        let success: SaiWsOnSuccess = Rc::new(|_: Option<Box<dyn Any>>, _: &mut App| {});
        let failure: SaiWsOnError = Rc::new(|error: &anyhow::Error, cx: &mut App| {
            Self::log_network(format!("update_wallet_settings error: {error}"), cx);
        });
        SaiWs::request_slice(&self.update_wallet_sai, body, options, Some(success), Some(failure), cx);
    }
}

impl UserActionsStore {
    pub fn sync_profile_goals_action(&mut self, cx: &mut Context<Self>) {
        if !Self::is_authenticated(cx) || Self::active_profile_id(cx).is_none() {
            return;
        }

        let body = UserServicesStore::shared(cx).update(cx, |services, cx| services.build_sync_profile_goals_body(cx));
        let options = Self::mutation_options(rise_wire_methods::user::SYNC_PROFILE_GOALS);

        let slice = Rc::clone(&self.sync_profile_goals_sai);
        let success: SaiWsOnSuccess = Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| {
            let data = slice.data();
            UserServicesStore::shared(cx).update(cx, |services, cx| services.handle_sync_profile_goals_success(data, cx));
        });
        let failure: SaiWsOnError = Rc::new(|error: &anyhow::Error, cx: &mut App| {
            UserServicesStore::shared(cx).update(cx, |services, cx| services.handle_sync_profile_goals_error(error, cx));
        });
        SaiWs::request_slice(&self.sync_profile_goals_sai, body, options, Some(success), Some(failure), cx);
    }

    pub fn sync_profile_plans_action(&mut self, cx: &mut Context<Self>) {
        if !Self::is_authenticated(cx) || Self::active_profile_id(cx).is_none() {
            return;
        }

        let body = UserServicesStore::shared(cx).update(cx, |services, cx| services.build_sync_profile_plans_body(cx));
        let options = Self::mutation_options(rise_wire_methods::user::SYNC_PROFILE_PLANS);

        let slice = Rc::clone(&self.sync_profile_plans_sai);
        let success: SaiWsOnSuccess = Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| {
            let data = slice.data();
            UserServicesStore::shared(cx).update(cx, |services, cx| services.handle_sync_profile_plans_success(data, cx));
        });
        let failure: SaiWsOnError = Rc::new(|error: &anyhow::Error, cx: &mut App| {
            UserServicesStore::shared(cx).update(cx, |services, cx| services.handle_sync_profile_plans_error(error, cx));
        });
        SaiWs::request_slice(&self.sync_profile_plans_sai, body, options, Some(success), Some(failure), cx);
    }

    pub fn set_mute_all_chats_action(&mut self, mute_all: bool, cx: &mut Context<Self>) {
        if (user_guide_sandbox_hooks().set_practice_mute_all)(mute_all, cx) {
            return;
        }
        if Self::active_profile_id(cx).is_none() {
            return;
        }

        let services = UserServicesStore::shared(cx);
        let previous = services.read(cx).mute_all_chats_enabled();
        services.update(cx, |services, cx| services.set_mute_all_chats_enabled(mute_all, cx));

        let body = rise_request::user::SetMuteAllChats::new(mute_all);
        let options = Self::mutation_options(rise_wire_methods::user::SET_MUTE_ALL_CHATS);

        let failure: SaiWsOnError = Rc::new(move |_: &anyhow::Error, cx: &mut App| {
            UserServicesStore::shared(cx).update(cx, |services, cx| services.set_mute_all_chats_enabled(previous, cx));
        });
        SaiWs::request_slice(&self.set_mute_all_chats_sai, body, options, None, Some(failure), cx);
    }

    pub fn get_chat_mute_overrides_action(&mut self, cx: &mut Context<Self>) {
        if Self::active_profile_id(cx).is_none() {
            return;
        }

        let body = rise_request::user::GetChatMuteOverrides {};
        let options = Self::mutation_options(rise_wire_methods::user::GET_CHAT_MUTE_OVERRIDES);

        let slice = Rc::clone(&self.get_chat_mute_overrides_sai);
        let success: SaiWsOnSuccess = Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| {
            let data = slice.data();
            UserServicesStore::shared(cx).update(cx, |services, cx| services.handle_chat_mute_overrides_success(data, cx));
        });
        SaiWs::request_slice(&self.get_chat_mute_overrides_sai, body, options, Some(success), None, cx);
    }

    pub fn add_chat_mute_override_action(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        if (user_guide_sandbox_hooks().set_practice_mute_exception)(chat_id, true, cx) {
            return;
        }
        if Self::active_profile_id(cx).is_none() {
            return;
        }

        UserServicesStore::shared(cx).update(cx, |services, cx| {
            services.apply_chat_mute_override_optimistic(chat_id, true, cx);
            services.mark_chat_mute_override_pending(chat_id, cx);
        });

        let body = rise_request::user::AddChatMuteOverride::new(chat_id.to_owned());
        let options = Self::mutation_options(rise_wire_methods::user::ADD_CHAT_MUTE_OVERRIDE);
        let (success, failure) = Self::chat_mute_override_callbacks(chat_id, true);
        SaiWs::request_slice(&self.add_chat_mute_override_sai, body, options, Some(success), Some(failure), cx);
    }

    pub fn remove_chat_mute_override_action(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        if (user_guide_sandbox_hooks().set_practice_mute_exception)(chat_id, false, cx) {
            return;
        }
        if Self::active_profile_id(cx).is_none() {
            return;
        }

        UserServicesStore::shared(cx).update(cx, |services, cx| {
            services.apply_chat_mute_override_optimistic(chat_id, false, cx);
            services.mark_chat_mute_override_pending(chat_id, cx);
        });

        let body = rise_request::user::RemoveChatMuteOverride::new(chat_id.to_owned());
        let options = Self::mutation_options(rise_wire_methods::user::REMOVE_CHAT_MUTE_OVERRIDE);
        let (success, failure) = Self::chat_mute_override_callbacks(chat_id, false);
        SaiWs::request_slice(&self.remove_chat_mute_override_sai, body, options, Some(success), Some(failure), cx);
    }

    fn chat_mute_override_callbacks(chat_id: &str, add: bool) -> (SaiWsOnSuccess, SaiWsOnError) {
        let success_chat_id = chat_id.to_owned();
        let success: SaiWsOnSuccess = Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| {
            UserServicesStore::shared(cx).update(cx, |services, cx| {
                services.clear_chat_mute_override_pending(&success_chat_id, cx);
            });
        });
        let failure_chat_id = chat_id.to_owned();
        let failure: SaiWsOnError = Rc::new(move |_: &anyhow::Error, cx: &mut App| {
            UserServicesStore::shared(cx).update(cx, |services, cx| {
                services.apply_chat_mute_override_optimistic(&failure_chat_id, !add, cx);
                services.clear_chat_mute_override_pending(&failure_chat_id, cx);
            });
        });
        (success, failure)
    }
}
