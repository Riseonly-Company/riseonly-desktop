use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::rc::Rc;

use gpui::{App, AsyncApp, Context, Task};
use rise_canon::RiseEvent;
use rise_canon::rise_key::{bot as bot_key, message as message_key, user as user_key};
use rise_canon::rise_service_text::user_service;
use rise_chat::MessageContentType as ChatMessageContentType;
use rise_core::foundation::foundation_date::{RiseClock, TimeInterval};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_engine::rise_wire::rise_json_value::RiseJSONValue;
use rise_events::name;
use rise_i18n::{FormatArg, tr, tr_args};
use rise_platform::file_manager::FileManager;
use serde_json::{Map, Value};

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::any_dictionary_bridge::AnyDictionaryBridge;
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::core::utilities::sai_cache::SaiCache;
use crate::core::lib::sai::core::utilities::sai_cache_cleaner::SaiCacheCleanLayer;
use crate::core::lib::sai::core::utilities::sai_payload_path::SaiJSONCodec;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiSliceState;
use crate::core::lib::sai::modules::sai_updater::sai_updater::{SaiUpdater, SaiUpdaterWriteScope};
use crate::core::lib::sai::modules::sai_ws::rise_event_payload::RiseEventPayload;
use crate::core::lib::sai::modules::sai_ws::sai_ws_chunked_file_upload::SaiWsChunkedFileUpload;
use crate::core::lib::sai::modules::sai_ws::sai_ws_disk_cache::SaiWsDiskCache;
use crate::core::lib::sai::modules::sai_ws::sai_ws_optimistic_update::SaiWsOptimisticLocalFile;
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{SaiWsCachePreferredLayer, SaiWsDiskCachePolicy};
use crate::core::lib::sai::modules::sai_ws::sai_ws_response_cache::SaiWsResponseCache;
use crate::core::lib::sai::sai::Sai;
use crate::core::push_notifications::push_deep_link_store::PushDeepLink;
use crate::core::widgets::notifier::notifier_store::{
    Notifier, NotifierAction, NotifierIcon, NotifierPosition, NotifierSendOptions, NotifierType,
};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::engine::core::rise_conversation_presentation::RiseConversationPresentation;
use crate::modules::chat::engine::core::rise_conversation_repository_facade::RiseConversationActionExecutor;
use crate::modules::chat::engine::core::rise_conversation_upload_support::RiseConversationUploadSupport;
use crate::modules::chat::engine::messages::rise_message_wire_codec::RiseMessageWireCodec;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatInfo, ChatType, User};
use crate::modules::chat::stores::chats::chat_interactions::chat_interactions::ChatInteractionsStore;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::chat::stores::message::message_actions::message_types::{
    GetMessageMessage, MessageContentType, MessageSystemInfo, MessageSystemKind,
};
use crate::modules::chat::stores::message::message_services::message_services::MessageServicesStore;
use crate::modules::media_picker::api::media_compression_configuration::MediaCompressionConfiguration;
use crate::modules::media_picker::export::outgoing_media_compressor::OutgoingMediaCompressor;
use crate::modules::media_picker::export::rise_outgoing_media_file::RiseOutgoingMediaFile;
use crate::modules::music::shared::utils::music::music_cover_upload::MusicCoverUpload;
use crate::modules::music::shared::utils::music::music_track_file_upload::MusicTrackFileUpload;
use crate::modules::post::stores::post_actions::post_types::AuthorInfoMore;
use crate::modules::user::engine::core::rise_user_block_wire_models::RiseBlockScope;
use crate::modules::user::engine::rise_user_block_presentation::{RiseUserBlockPresentation, RiseUserBlockSnapshot};
use crate::modules::user::stores::user::user_actions::user_actions::UserActionsStore;
use crate::modules::user::stores::user::user_actions::user_contacts_actions::UserContactCallback;
use crate::modules::user::stores::user::user_actions::user_contact_types::{
    UserContactBlockFact, UserContactDraft, UserContactLimits, UserContactMutationError, UserContactPeer,
    UserContactPhotoUpload,
};
use crate::modules::user::stores::user::user_actions::user_contact_wire_types::{
    ContactJoinEventDtoExt, SavedContactDtoExt, SyncUserContactsGatewayDTO, UserContactJoinEventWireDTO,
    UserContactJoinEventsGatewayDTO, UserContactPhotoSuggestionWireDTO, UserContactPhotoSuggestionsListGatewayDTO,
    UserContactSyncSettingsGatewayDTO, UserContactSyncSettingsWireDTO, UserContactUserWireDTO,
    UserSavedContactGatewayDTO, UserSavedContactWireDTO, UserSavedContactsListGatewayDTO,
};
use crate::modules::user::stores::user::user_actions::user_wire_types::{
    UserGetProfileByTagGatewayDTO, UserGetProfileByTagGatewayDTOExt,
};
use crate::modules::user::stores::user::user_services::user_guide_sandbox_hooks::user_guide_sandbox_hooks;
use crate::modules::user::stores::user::user_services::user_services::UserServicesStore;

pub type UserSavedContactsIndex = Rc<(Vec<UserSavedContactWireDTO>, HashMap<String, UserSavedContactWireDTO>)>;

impl UserServicesStore {
    pub const CONTACT_JOIN_VIRTUAL_CHAT_PREFIX: &'static str = "contact-join:";

    pub fn contact_join_virtual_chat_id(event_id: &str) -> String {
        format!("{}{event_id}", Self::CONTACT_JOIN_VIRTUAL_CHAT_PREFIX)
    }

    pub fn is_contact_join_virtual_chat_id(id: &str) -> bool {
        id.starts_with(Self::CONTACT_JOIN_VIRTUAL_CHAT_PREFIX)
    }

    pub fn contact_join_local_message_id(event_id: &str) -> String {
        format!("contact-join-event:{event_id}")
    }

    pub fn contact_sync_settings_cache_key(user_id: &str) -> String {
        SaiCache::new(["contactSyncSettings", user_id]).key
    }

    pub fn saved_contact_cache_key(user_id: &str, target_user_id: &str) -> String {
        SaiCache::new(["savedContact", user_id, target_user_id]).key
    }

    pub fn saved_contacts_list_cache_key(user_id: &str) -> String {
        SaiCache::new(["savedContactsMemoryV2", user_id]).key
    }

    pub fn legacy_saved_contacts_list_cache_key(user_id: &str) -> String {
        SaiCache::new(["savedContacts", user_id]).key
    }

    pub fn contact_join_events_cache_key(user_id: &str) -> String {
        SaiCache::new(["contactJoinEvents", user_id]).key
    }

    pub fn contact_photo_suggestions_cache_key(user_id: &str) -> String {
        SaiCache::new(["contactPhotoSuggestions", user_id]).key
    }

    fn contact_wire_user_id(cx: &mut App) -> Option<String> {
        AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire()
    }

    fn contact_observed_payload(cache_key: &str, cx: &mut App) -> Option<Value> {
        SaiWsResponseCache::observed_payload(
            cache_key,
            SaiWsCachePreferredLayer::MemoryFirst,
            SaiWsDiskCachePolicy::PreserveExisting,
            cx,
        )
    }

    fn contact_current_payload(cache_key: &str, cx: &mut App) -> Option<Value> {
        SaiWsResponseCache::current_payload(
            cache_key,
            SaiWsCachePreferredLayer::MemoryFirst,
            SaiWsDiskCachePolicy::PreserveExisting,
            cx,
        )
    }

    fn contact_replace_root(cache_key: &str, root: Map<String, Value>, cx: &mut App) {
        let _ = SaiUpdater::replace_root(cache_key, root, SaiUpdaterWriteScope::Cache, cx);
    }

    pub fn purge_legacy_saved_contacts_disk_cache(&mut self, user_id: &str, cx: &mut Context<Self>) {
        let legacy_key = Self::legacy_saved_contacts_list_cache_key(user_id);
        SaiWsResponseCache::remove(&legacy_key, SaiCacheCleanLayer::Memory, cx);
        drop(RiseAccountRuntime::current().spawn_blocking(move || {
            SaiWsDiskCache::remove(&legacy_key);
        }));
    }

    pub fn upload_contact_photo(
        &mut self,
        local_file: SaiWsOptimisticLocalFile,
        request_id: String,
        owner_id: String,
        cx: &mut Context<Self>,
    ) -> Task<anyhow::Result<UserContactPhotoUpload>> {
        let normalized_owner_id = trimming_whitespaces_and_newlines(&owner_id).to_lowercase();
        if normalized_owner_id.is_empty() {
            return Task::ready(Err(UserContactMutationError::InvalidContact.into()));
        }
        let upload_resource_id = format!("{normalized_owner_id}:{}", request_id.to_lowercase());
        cx.spawn(async move |_, cx| {
            let compressed = OutgoingMediaCompressor::compress_if_needed_optimistic(
                local_file.clone(),
                None,
                MediaCompressionConfiguration::avatar(),
                None,
                None,
            )
            .await?;
            let generated_temporary_path = (compressed.url.standardized() != local_file.url.standardized())
                .then(|| compressed.url.to_file_path())
                .flatten();
            let original_path = local_file.url.to_file_path().unwrap_or_default();
            let prepared = RiseConversationUploadSupport::prepared_local_file(
                &RiseOutgoingMediaFile::new(original_path, local_file.filename.clone(), local_file.content_type.clone()),
                &compressed,
            );
            let compressed_path = compressed.url.to_file_path().unwrap_or_default();
            let upload = cx.update(|cx| {
                SaiWsChunkedFileUpload::upload(
                    compressed_path,
                    prepared.filename.clone(),
                    compressed.content_type.clone(),
                    "contact_photo".to_owned(),
                    Some(upload_resource_id),
                    SaiAccountCacheScope::active_user_id(),
                    SaiAccountCacheScope::active_epoch(),
                    None,
                    None,
                    cx,
                )
            });
            let done = upload.await;
            if let Some(path) = generated_temporary_path {
                let _ = FileManager.remove_item(&path);
            }
            let done = done?;
            let (Some(asset_id), Some(stable_url)) =
                (MusicTrackFileUpload::remote_asset_id(&done), MusicCoverUpload::extract_remote_url(&done))
            else {
                return Err(UserContactMutationError::PhotoUploadResponse.into());
            };
            Ok(UserContactPhotoUpload { asset_id, stable_file_url: stable_url })
        })
    }

    pub fn contact_sync_settings(&self, cx: &mut App) -> Option<UserContactSyncSettingsWireDTO> {
        let user_id = Self::contact_wire_user_id(cx)?;
        let payload = Self::contact_observed_payload(&Self::contact_sync_settings_cache_key(&user_id), cx)?;
        let gateway = SaiJSONCodec::decode::<UserContactSyncSettingsGatewayDTO>(Some(&payload))?;
        gateway.settings
    }

    pub fn contact_sync_enabled(&self, cx: &mut App) -> bool {
        self.contact_sync_settings(cx).is_none_or(|settings| settings.sync_enabled)
    }

    pub fn contact_sync_batch_size(&self, cx: &mut App) -> i64 {
        let fallback = UserContactLimits::fallback_sync_batch();
        let requested = self.contact_sync_settings(cx).map_or(fallback, |settings| settings.max_batch_size);
        fallback.min(requested.max(1))
    }

    pub fn saved_contact_gateway(&self, target_user_id: &str, cx: &mut App) -> Option<UserSavedContactGatewayDTO> {
        Self::saved_contact_gateway_projection(target_user_id, cx)
    }

    fn saved_contact_gateway_projection(target_user_id: &str, cx: &mut App) -> Option<UserSavedContactGatewayDTO> {
        let user_id = Self::contact_wire_user_id(cx)?;
        let key = Self::saved_contact_cache_key(&user_id, target_user_id);
        SaiWsResponseCache::observed_projection(
            &key,
            "user.saved-contact.gateway.v1",
            SaiWsCachePreferredLayer::MemoryFirst,
            SaiWsDiskCachePolicy::PreserveExisting,
            |payload| SaiJSONCodec::decode::<UserSavedContactGatewayDTO>(payload),
            cx,
        )
    }

    pub fn saved_contact(&self, target_user_id: &str, cx: &mut App) -> Option<UserSavedContactWireDTO> {
        Self::observed_saved_contact(target_user_id, cx)
    }

    /// saved contact of one person without touching the users store entity
    pub fn observed_saved_contact(target_user_id: &str, cx: &mut App) -> Option<UserSavedContactWireDTO> {
        if let Some(gateway) = Self::saved_contact_gateway_projection(target_user_id, cx) {
            return if gateway.is_contact { gateway.contact } else { None };
        }
        Self::saved_contacts_index_projection(cx).1.get(target_user_id).cloned()
    }

    pub fn is_saved_contact_loaded(&self, target_user_id: &str, cx: &mut App) -> bool {
        if (user_guide_sandbox_hooks().owns_user_id)(target_user_id, cx) {
            return true;
        }
        if self.saved_contact_gateway(target_user_id, cx).is_some() {
            return true;
        }
        if self.saved_contacts_index(cx).0.iter().any(|contact| contact.target_user_id == target_user_id) {
            return true;
        }
        let Some(user_id) = Self::contact_wire_user_id(cx) else {
            return false;
        };
        let Some(payload) = Self::contact_observed_payload(&Self::saved_contacts_list_cache_key(&user_id), cx) else {
            return false;
        };
        let Some(page) = SaiJSONCodec::decode::<UserSavedContactsListGatewayDTO>(Some(&payload)) else {
            return false;
        };
        !page.has_more
    }

    pub fn saved_contacts(&self, cx: &mut App) -> Vec<UserSavedContactWireDTO> {
        self.saved_contacts_index(cx).0.clone()
    }

    fn saved_contacts_index(&self, cx: &mut App) -> UserSavedContactsIndex {
        Self::saved_contacts_index_projection(cx)
    }

    fn saved_contacts_index_projection(cx: &mut App) -> UserSavedContactsIndex {
        let Some(user_id) = Self::contact_wire_user_id(cx) else {
            return Rc::new((Vec::new(), HashMap::new()));
        };
        let key = Self::saved_contacts_list_cache_key(&user_id);
        SaiWsResponseCache::observed_projection(
            &key,
            "user.saved-contacts.index.v1",
            SaiWsCachePreferredLayer::MemoryFirst,
            SaiWsDiskCachePolicy::PreserveExisting,
            |payload| {
                let ordered = SaiJSONCodec::decode::<UserSavedContactsListGatewayDTO>(payload)
                    .map(|dto| dto.contacts)
                    .unwrap_or_default();
                let mut by_target_id: HashMap<String, UserSavedContactWireDTO> = HashMap::with_capacity(ordered.len());
                for contact in &ordered {
                    by_target_id.entry(contact.target_user_id.clone()).or_insert_with(|| contact.clone());
                }
                Rc::new((ordered, by_target_id))
            },
            cx,
        )
    }

    pub fn saved_contact_target_ids(&self, cx: &mut App) -> HashSet<String> {
        self.saved_contacts_index(cx).0.iter().map(|contact| contact.target_user_id.clone()).collect()
    }

    pub fn hydrate_saved_contact_details_from_list(&mut self, cx: &mut Context<Self>) {
        let Some(user_id) = Self::contact_wire_user_id(cx) else {
            return;
        };
        for contact in self.saved_contacts(cx) {
            let target_user_id = contact.target_user_id.clone();
            Self::write_saved_contact_gateway(
                &UserSavedContactGatewayDTO {
                    contact: Some(contact),
                    is_contact: true,
                    success: true,
                    ..Default::default()
                },
                &user_id,
                &target_user_id,
                cx,
            );
        }
    }

    pub fn replace_saved_contacts_from_completed_sync(
        &mut self,
        response: &SyncUserContactsGatewayDTO,
        user_id: &str,
        cx: &mut Context<Self>,
    ) -> bool {
        if !response.sync_completed {
            return false;
        }
        let root = UserSavedContactsListGatewayDTO {
            contacts: response.matched_contacts.clone(),
            has_more: false,
            relative_id: None,
            success: true,
            ..Default::default()
        };
        let Some(encoded) = SaiJSONCodec::encode(&root) else {
            return false;
        };
        Self::contact_replace_root(&Self::saved_contacts_list_cache_key(user_id), encoded, cx);
        self.hydrate_saved_contact_details_from_list(cx);
        true
    }

    fn saved_contacts_list_page(cx: &mut App) -> Option<UserSavedContactsListGatewayDTO> {
        let user_id = Self::contact_wire_user_id(cx)?;
        let payload = Self::contact_observed_payload(&Self::saved_contacts_list_cache_key(&user_id), cx)?;
        SaiJSONCodec::decode::<UserSavedContactsListGatewayDTO>(Some(&payload))
    }

    pub fn saved_contacts_has_more(&self, cx: &mut App) -> bool {
        Self::saved_contacts_list_page(cx).is_some_and(|dto| dto.has_more)
    }

    pub fn saved_contacts_relative_id(&self, cx: &mut App) -> Option<String> {
        Self::saved_contacts_list_page(cx).and_then(|dto| dto.relative_id)
    }

    pub fn profile_block_fact(&self, target_user_id: &str, cx: &mut App) -> UserContactBlockFact {
        let Some(response) = self.foreign_profile_payload(target_user_id, cx) else {
            return UserContactBlockFact::Unknown;
        };
        let raw = response.resolved_viewer_block_scope().map(|raw| trimming_whitespaces_and_newlines(raw).to_lowercase());
        if let Some(raw) = raw.as_deref()
            && !raw.is_empty()
        {
            return UserContactBlockFact::Resolved(Some(
                RiseBlockScope::from_raw_value(raw).unwrap_or(RiseBlockScope::Standard),
            ));
        }
        if let Some(blocked) = response.resolved_viewer_has_blocked() {
            return UserContactBlockFact::Resolved(blocked.then_some(RiseBlockScope::Standard));
        }
        if raw.is_none() { UserContactBlockFact::Unknown } else { UserContactBlockFact::Resolved(None) }
    }

    pub fn owner_block_scope(&self, target_user_id: &str, cx: &mut App) -> Option<RiseBlockScope> {
        let target = trimming_whitespaces_and_newlines(target_user_id);
        if target.is_empty() {
            return None;
        }
        let response = self.foreign_profile_payload(target, cx)?;
        let raw = trimming_whitespaces_and_newlines(response.resolved_owner_block_scope()?).to_lowercase();
        if raw.is_empty() {
            return None;
        }

        Some(RiseBlockScope::from_raw_value(&raw).unwrap_or(RiseBlockScope::Standard))
    }

    fn block_snapshot(cx: &App) -> Option<&RiseUserBlockSnapshot> {
        rise_ui::shared::try_read::<RiseUserBlockPresentation>(cx).map(|presentation| presentation.snapshot())
    }

    pub fn viewer_block_scope(&self, target_user_id: &str, cx: &mut App) -> Option<RiseBlockScope> {
        let target = trimming_whitespaces_and_newlines(target_user_id);
        if target.is_empty() {
            return None;
        }
        let hooks = user_guide_sandbox_hooks();
        if (hooks.owns_user_id)(target, cx) {
            return (hooks.panda_block_scope)(cx);
        }
        if let Some(scope) = Self::block_snapshot(cx).and_then(|snapshot| snapshot.scope(target)) {
            return Some(scope);
        }
        match self.profile_block_fact(target, cx) {
            UserContactBlockFact::Resolved(scope) => scope,
            UserContactBlockFact::Unknown => None,
        }
    }

    pub fn viewer_has_blocked(&self, target_user_id: &str, cx: &mut App) -> Option<bool> {
        let target = trimming_whitespaces_and_newlines(target_user_id);
        if target.is_empty() {
            return None;
        }
        let hooks = user_guide_sandbox_hooks();
        if (hooks.owns_user_id)(target, cx) {
            return Some((hooks.panda_block_scope)(cx).is_some());
        }
        if Self::block_snapshot(cx).and_then(|snapshot| snapshot.scope(target)).is_some() {
            return Some(true);
        }
        match self.profile_block_fact(target, cx) {
            UserContactBlockFact::Resolved(scope) => Some(scope.is_some()),
            UserContactBlockFact::Unknown => None,
        }
    }

    pub fn viewer_has_blocked_from_snapshot(&self, user_id: &str, cx: &App) -> bool {
        let target = trimming_whitespaces_and_newlines(user_id);
        if target.is_empty() {
            return false;
        }
        Self::block_snapshot(cx).is_some_and(|snapshot| snapshot.is_blocked(target))
    }

    pub fn is_totally_blocked(&self, user_id: &str, cx: &App) -> bool {
        let target = trimming_whitespaces_and_newlines(user_id);
        if target.is_empty() {
            return false;
        }
        Self::block_snapshot(cx).is_some_and(|snapshot| snapshot.is_totally_blocked(target))
    }

    pub fn blocked_users(&self, cx: &mut App) -> RiseUserBlockSnapshot {
        RiseUserBlockPresentation::shared(cx).read(cx).snapshot().clone()
    }

    pub fn contact_sync_settings_snapshot(&self, user_id: &str, cx: &mut App) -> Option<Map<String, Value>> {
        let payload = Self::contact_observed_payload(&Self::contact_sync_settings_cache_key(user_id), cx)?;
        AnyDictionaryBridge::dictionary(Some(&payload))
    }

    pub fn apply_contact_sync_setting_optimistic(&mut self, user_id: &str, enabled: bool, cx: &mut Context<Self>) {
        let current = self.contact_sync_settings(cx).unwrap_or_else(|| UserContactSyncSettingsWireDTO {
            max_batch_size: UserContactLimits::fallback_sync_batch(),
            sync_enabled: true,
            ..Default::default()
        });
        let settings = UserContactSyncSettingsWireDTO { sync_enabled: enabled, ..current };
        Self::replace_contact_sync_settings(
            &UserContactSyncSettingsGatewayDTO { settings: Some(settings), success: true, ..Default::default() },
            user_id,
            cx,
        );
    }

    pub fn restore_contact_sync_settings(
        &mut self,
        snapshot: Option<Map<String, Value>>,
        user_id: &str,
        cx: &mut Context<Self>,
    ) {
        let key = Self::contact_sync_settings_cache_key(user_id);
        match snapshot {
            Some(snapshot) => Self::contact_replace_root(&key, snapshot, cx),
            None => SaiWsResponseCache::remove(&key, SaiCacheCleanLayer::Memory, cx),
        }
    }

    pub fn reconcile_contact_sync_settings(
        &mut self,
        gateway: &UserContactSyncSettingsGatewayDTO,
        user_id: &str,
        cx: &mut Context<Self>,
    ) {
        self.contact_sync_settings_did_load = gateway.settings.is_some();
        self.contact_sync_settings_error = (!gateway.error.is_empty()).then(|| gateway.error.clone());
        cx.notify();
        Self::replace_contact_sync_settings(gateway, user_id, cx);
    }

    pub fn contact_sync_settings_failed(&mut self, error: &anyhow::Error, cx: &mut Context<Self>) {
        self.contact_sync_settings_error = Some(error.to_string());
        cx.notify();
    }

    pub fn saved_contact_snapshot(&self, target_user_id: &str, cx: &mut App) -> Option<Map<String, Value>> {
        let user_id = Self::contact_wire_user_id(cx)?;
        let key = Self::saved_contact_cache_key(&user_id, target_user_id);
        let payload = Self::contact_observed_payload(&key, cx)?;
        AnyDictionaryBridge::dictionary(Some(&payload))
    }

    pub fn apply_saved_contact_optimistic(
        &mut self,
        draft: &UserContactDraft,
        peer: &UserContactPeer,
        cx: &mut Context<Self>,
    ) {
        let Some(user_id) = Self::contact_wire_user_id(cx) else {
            return;
        };
        let previous = self.saved_contact(&draft.target_user_id, cx);
        let now = RiseClock::now().ms();
        let wire_user = UserContactUserWireDTO {
            id: peer.id.clone(),
            is_bot: peer.is_bot,
            is_online: peer.is_online,
            is_premium: peer.is_premium,
            last_seen: peer.last_seen,
            logo: peer.logo.clone(),
            name: peer.name.clone(),
            tag: peer.tag.clone(),
            ..Default::default()
        };
        let first_name = draft.normalized_first_name();
        let last_name = draft.normalized_last_name();
        let display_name = [first_name.as_str(), last_name.as_str()]
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        let contact = UserSavedContactWireDTO {
            avatar_override_asset_id: previous.as_ref().and_then(|previous| previous.avatar_override_asset_id.clone()),
            avatar_override_url: previous.as_ref().and_then(|previous| previous.avatar_override_url.clone()),
            created_at_ms: previous.as_ref().map_or(now, |previous| previous.created_at_ms),
            display_name,
            first_name,
            id: previous
                .as_ref()
                .map_or_else(|| format!("optimistic-{}", draft.target_user_id), |previous| previous.id.clone()),
            last_name,
            note: draft.normalized_note(),
            share_phone_number: draft.share_phone_number,
            source: previous.as_ref().map_or_else(
                || user_service::SAVED_CONTACT_SOURCE_MANUAL.to_owned(),
                |previous| previous.source.clone(),
            ),
            sync_to_phone: draft.sync_to_phone,
            target_user_id: draft.target_user_id.clone(),
            updated_at_ms: now,
            user: previous.as_ref().and_then(|previous| previous.user.clone()).or(Some(wire_user)),
        };
        let gateway = UserSavedContactGatewayDTO {
            contact: Some(contact.clone()),
            is_contact: true,
            success: true,
            ..Default::default()
        };
        Self::write_saved_contact_gateway(&gateway, &user_id, &draft.target_user_id, cx);
        Self::upsert_saved_contact_list(&contact, &user_id, cx);
        self.saved_contact_pending_targets.insert(draft.target_user_id.clone());
        self.saved_contact_failed_targets.remove(&draft.target_user_id);
        cx.notify();
    }

    pub fn apply_delete_saved_contact_optimistic(&mut self, target_user_id: &str, cx: &mut Context<Self>) {
        let Some(user_id) = Self::contact_wire_user_id(cx) else {
            return;
        };
        let contact = self.saved_contact(target_user_id, cx);
        Self::write_saved_contact_gateway(
            &UserSavedContactGatewayDTO { contact, is_contact: false, success: true, ..Default::default() },
            &user_id,
            target_user_id,
            cx,
        );
        Self::remove_saved_contact_from_list(target_user_id, &user_id, cx);
        self.saved_contact_pending_targets.insert(target_user_id.to_owned());
        self.saved_contact_failed_targets.remove(target_user_id);
        cx.notify();
    }

    pub fn restore_saved_contact(
        &mut self,
        snapshot: Option<Map<String, Value>>,
        target_user_id: &str,
        error: &anyhow::Error,
        cx: &mut Context<Self>,
    ) {
        let Some(user_id) = Self::contact_wire_user_id(cx) else {
            return;
        };
        let key = Self::saved_contact_cache_key(&user_id, target_user_id);
        match snapshot {
            Some(snapshot) => {
                let gateway = SaiJSONCodec::decode::<UserSavedContactGatewayDTO>(Some(&Value::Object(snapshot.clone())));
                Self::contact_replace_root(&key, snapshot, cx);
                if let Some(gateway) = gateway
                    && gateway.is_contact
                    && let Some(contact) = gateway.contact
                {
                    Self::upsert_saved_contact_list(&contact, &user_id, cx);
                }
            }
            None => {
                SaiWsResponseCache::remove(&key, SaiCacheCleanLayer::Memory, cx);
                Self::remove_saved_contact_from_list(target_user_id, &user_id, cx);
            }
        }
        self.saved_contact_pending_targets.remove(target_user_id);
        self.saved_contact_failed_targets.insert(target_user_id.to_owned());
        cx.notify();
        let reason = error.to_string();
        Sai::log("User", move || format!("saved contact mutation failed: {reason}"), SaiLogKind::Network, cx);
    }

    pub fn reconcile_saved_contact(
        &mut self,
        gateway: &UserSavedContactGatewayDTO,
        target_user_id: &str,
        cx: &mut Context<Self>,
    ) {
        let Some(user_id) = Self::contact_wire_user_id(cx) else {
            return;
        };
        Self::write_saved_contact_gateway(gateway, &user_id, target_user_id, cx);
        match gateway.contact.as_ref().filter(|_| gateway.is_contact) {
            Some(contact) => Self::upsert_saved_contact_list(contact, &user_id, cx),
            None => Self::remove_saved_contact_from_list(target_user_id, &user_id, cx),
        }
        self.saved_contact_pending_targets.remove(target_user_id);
        self.saved_contact_failed_targets.remove(target_user_id);
        cx.notify();
    }

    pub fn saved_contact_fetch_failed(&mut self, target_user_id: &str, error: &anyhow::Error, cx: &mut Context<Self>) {
        self.saved_contact_failed_targets.insert(target_user_id.to_owned());
        cx.notify();
        let reason = error.to_string();
        Sai::log("User", move || format!("get_saved_contact failed: {reason}"), SaiLogKind::Network, cx);
    }

    pub fn reconcile_contact_metadata_from_foreign_profile(
        &mut self,
        response: &UserGetProfileByTagGatewayDTO,
        target_user_id: &str,
        cx: &mut Context<Self>,
    ) {
        let profile = response.profile.as_ref();
        let has_metadata = profile.is_some_and(|profile| profile.is_saved_contact.is_some() || profile.saved_contact.is_some());
        if !has_metadata {
            return;
        }
        let Some(user_id) = Self::contact_wire_user_id(cx) else {
            return;
        };
        let saved_contact = profile.and_then(|profile| profile.saved_contact.clone());
        let gateway = UserSavedContactGatewayDTO {
            contact: saved_contact.clone(),
            error: response.error.clone(),
            is_contact: profile
                .and_then(|profile| profile.is_saved_contact)
                .unwrap_or(saved_contact.is_some()),
            success: response.success,
        };
        Self::write_saved_contact_gateway(&gateway, &user_id, target_user_id, cx);
        if let Some(contact) = saved_contact {
            Self::upsert_saved_contact_list(&contact, &user_id, cx);
        }
    }

    pub fn block_projection_cache_keys(&self, target_user_id: &str, cx: &mut App) -> Vec<String> {
        let mut keys = Vec::new();
        if let Some(viewer_id) = Self::contact_wire_user_id(cx) {
            keys.push(Self::profile_by_id_cache_key(target_user_id, &viewer_id));
        }
        if let Some(tag) = self.foreign_profile_tag(target_user_id, cx) {
            keys.push(Self::profile_by_tag_cache_key(&tag));
        }
        keys.into_iter().filter(|key| Self::contact_current_payload(key, cx).is_some()).collect()
    }

    pub fn block_projection_snapshot(&self, target_user_id: &str, cx: &mut App) -> HashMap<String, Map<String, Value>> {
        let mut snapshot = HashMap::new();
        for key in self.block_projection_cache_keys(target_user_id, cx) {
            let payload = Self::contact_current_payload(&key, cx);
            let Some(root) = AnyDictionaryBridge::dictionary(payload.as_ref()) else {
                continue;
            };
            snapshot.insert(key, root);
        }
        snapshot
    }

    fn patch_block_projection(&self, target_user_id: &str, scope: Option<RiseBlockScope>, cx: &mut App) {
        let mut fields = Map::new();
        fields.insert(user_key::VIEWER_HAS_BLOCKED.to_owned(), Value::Bool(scope.is_some()));
        fields.insert(
            user_key::VIEWER_BLOCK_SCOPE.to_owned(),
            Value::String(scope.map(|scope| scope.raw_value().to_owned()).unwrap_or_default()),
        );
        for key in self.block_projection_cache_keys(target_user_id, cx) {
            let _ = SaiUpdater::patch_root(&key, &fields, SaiUpdaterWriteScope::Cache, cx);
        }
    }

    pub fn apply_blocked_optimistic(
        &mut self,
        target_user_id: &str,
        scope: Option<RiseBlockScope>,
        cx: &mut Context<Self>,
    ) {
        self.patch_block_projection(target_user_id, scope, cx);
        self.contact_block_pending_targets.insert(target_user_id.to_owned());
        cx.notify();
    }

    pub fn restore_blocked(
        &mut self,
        snapshot: HashMap<String, Map<String, Value>>,
        target_user_id: &str,
        error: &anyhow::Error,
        cx: &mut Context<Self>,
    ) {
        self.contact_block_pending_targets.remove(target_user_id);
        cx.notify();
        for (key, root) in snapshot {
            Self::contact_replace_root(&key, root, cx);
        }
        let reason = error.to_string();
        Sai::log("User", move || format!("contact block mutation failed: {reason}"), SaiLogKind::Network, cx);
    }

    pub fn reconcile_blocked(&mut self, target_user_id: &str, scope: Option<RiseBlockScope>, cx: &mut Context<Self>) {
        self.patch_block_projection(target_user_id, scope, cx);
        self.contact_block_pending_targets.remove(target_user_id);
        cx.notify();
    }

    pub fn apply_saved_contact_photo_optimistic(
        &mut self,
        target_user_id: &str,
        _upload: Option<&UserContactPhotoUpload>,
        cx: &mut Context<Self>,
    ) -> Option<Map<String, Value>> {
        let snapshot = self.saved_contact_snapshot(target_user_id, cx);
        let Some(user_id) = Self::contact_wire_user_id(cx) else {
            return snapshot;
        };
        let Some(current) = self.saved_contact(target_user_id, cx) else {
            return snapshot;
        };
        let updated = UserSavedContactWireDTO { updated_at_ms: RiseClock::now().ms(), ..current };
        let gateway = UserSavedContactGatewayDTO {
            contact: Some(updated.clone()),
            is_contact: true,
            success: true,
            ..Default::default()
        };
        Self::write_saved_contact_gateway(&gateway, &user_id, target_user_id, cx);
        Self::upsert_saved_contact_list(&updated, &user_id, cx);
        self.contact_photo_pending_targets.insert(target_user_id.to_owned());
        cx.notify();
        snapshot
    }

    pub fn reconcile_saved_contact_photo(
        &mut self,
        gateway: &UserSavedContactGatewayDTO,
        target_user_id: &str,
        cx: &mut Context<Self>,
    ) {
        self.reconcile_saved_contact(gateway, target_user_id, cx);
        self.contact_photo_pending_targets.remove(target_user_id);
        cx.notify();
    }

    pub fn restore_saved_contact_photo(
        &mut self,
        snapshot: Option<Map<String, Value>>,
        target_user_id: &str,
        error: &anyhow::Error,
        cx: &mut Context<Self>,
    ) {
        self.restore_saved_contact(snapshot, target_user_id, error, cx);
        self.contact_photo_pending_targets.remove(target_user_id);
        cx.notify();
    }

    pub fn contact_photo_suggestions(&self, cx: &mut App) -> Vec<UserContactPhotoSuggestionWireDTO> {
        let Some(user_id) = Self::contact_wire_user_id(cx) else {
            return Vec::new();
        };
        let Some(payload) = Self::contact_observed_payload(&Self::contact_photo_suggestions_cache_key(&user_id), cx)
        else {
            return Vec::new();
        };
        SaiJSONCodec::decode::<UserContactPhotoSuggestionsListGatewayDTO>(Some(&payload))
            .map(|dto| dto.suggestions)
            .unwrap_or_default()
            .into_iter()
            .filter(|suggestion| suggestion.status.to_uppercase() == user_service::CONTACT_PHOTO_SUGGESTION_STATUS_PENDING)
            .collect()
    }

    pub fn contact_photo_suggested_event_handler(&mut self, payload: Option<&Value>, cx: &mut Context<Self>) {
        let _ = payload;
        cx.defer(|cx| {
            UserActionsStore::shared(cx)
                .update(cx, |actions, cx| actions.list_contact_photo_suggestions_action(None, 0, cx));
        });
    }

    pub fn present_next_contact_photo_suggestion_if_needed(&mut self, cx: &mut Context<Self>) {
        if self.active_contact_photo_suggestion.is_some() || self.should_present_contact_sync_onboarding {
            return;
        }
        let mut suggestions = self.contact_photo_suggestions(cx);
        suggestions.sort_by_key(|suggestion| suggestion.created_at_ms);
        self.active_contact_photo_suggestion = suggestions.into_iter().next();
        cx.notify();
    }

    pub fn finish_presenting_contact_photo_suggestion(&mut self, cx: &mut Context<Self>) {
        self.active_contact_photo_suggestion = None;
        cx.notify();
        self.present_next_contact_photo_suggestion_if_needed(cx);
    }

    pub fn apply_contact_photo_suggestion_resolved_optimistic(
        &mut self,
        suggestion_id: &str,
        cx: &mut Context<Self>,
    ) -> Option<Map<String, Value>> {
        let user_id = Self::contact_wire_user_id(cx)?;
        let key = Self::contact_photo_suggestions_cache_key(&user_id);
        let snapshot = Self::contact_observed_payload(&key, cx).and_then(|payload| AnyDictionaryBridge::dictionary(Some(&payload)));
        let _ = SaiUpdater::remove_row(
            &key,
            user_key::SUGGESTIONS,
            user_key::ID,
            suggestion_id,
            SaiUpdaterWriteScope::Cache,
            cx,
        );
        snapshot
    }

    pub fn restore_contact_photo_suggestions(&mut self, snapshot: Option<Map<String, Value>>, cx: &mut Context<Self>) {
        let Some(user_id) = Self::contact_wire_user_id(cx) else {
            return;
        };
        let Some(snapshot) = snapshot else {
            return;
        };
        Self::contact_replace_root(&Self::contact_photo_suggestions_cache_key(&user_id), snapshot, cx);
    }

    pub fn contact_join_events(&self, cx: &mut App) -> Vec<UserContactJoinEventWireDTO> {
        let Some(user_id) = Self::contact_wire_user_id(cx) else {
            return Vec::new();
        };
        let Some(payload) = Self::contact_observed_payload(&Self::contact_join_events_cache_key(&user_id), cx) else {
            return Vec::new();
        };
        SaiJSONCodec::decode::<UserContactJoinEventsGatewayDTO>(Some(&payload))
            .map(|dto| dto.events)
            .unwrap_or_default()
    }

    pub fn contact_joined_event_handler(&mut self, payload: Option<&Value>, cx: &mut Context<Self>) {
        let Some(event) = Self::decode_contact_join_event(payload) else {
            return;
        };
        let Some(contact) = event.contact.as_ref() else {
            return;
        };
        if AuthServicesStore::shared(cx).read(cx).is_current_user(&contact.id) {
            return;
        }
        let contact_id = contact.id.clone();
        self.remember_contact_join_visible_name(payload, &contact_id);
        self.insert_contact_join_event(&event, cx);
        self.restore_contact_join_dialogs_from_inbox(cx);
        self.present_contact_join_event_if_needed(&event, cx);
        if !event.is_read {
            let event_ids = vec![event.id.clone()];
            cx.defer(move |cx| {
                UserActionsStore::shared(cx)
                    .update(cx, |actions, cx| actions.mark_contact_join_events_read_action(event_ids, cx));
            });
        }
    }

    fn recent_contact_join_events(&self, cx: &mut App) -> Vec<UserContactJoinEventWireDTO> {
        let mut events = self.contact_join_events(cx);
        events.sort_by(|lhs, rhs| rhs.created_at_ms.cmp(&lhs.created_at_ms));
        events.truncate(usize::try_from(UserContactLimits::maximum_join_virtual_dialogs()).unwrap_or(0));
        events
    }

    pub fn recent_unread_contact_join_event_ids(&mut self, cx: &mut Context<Self>) -> Vec<String> {
        self.snapshot_contact_join_events_cache_in_memory(cx);
        let unread: Vec<UserContactJoinEventWireDTO> = self
            .recent_contact_join_events(cx)
            .into_iter()
            .filter(|event| !event.is_read)
            .take(usize::try_from(UserContactLimits::maximum_join_ack_batch()).unwrap_or(0))
            .collect();
        let toasts = usize::try_from(UserContactLimits::maximum_join_toasts_per_refresh()).unwrap_or(0);
        for event in unread.iter().take(toasts) {
            self.restore_contact_join_virtual_dialog(event, cx);
            self.present_contact_join_event_if_needed(event, cx);
        }
        unread.into_iter().map(|event| event.id).collect()
    }

    fn chat_infos(cx: &mut App) -> Vec<ChatInfo> {
        ChatServicesStore::shared(cx).update(cx, |chats, cx| chats.current_chat_infos(cx))
    }

    fn drop_virtual_chats(virtual_ids: &HashSet<String>, cx: &mut App) {
        let remaining: Vec<ChatInfo> =
            Self::chat_infos(cx).into_iter().filter(|chat| !virtual_ids.contains(&chat.id)).collect();
        ChatServicesStore::shared(cx).update(cx, |chats, cx| chats.restore_chats_snapshot(remaining, cx));
        for id in virtual_ids {
            SaiWsResponseCache::remove(
                &MessageServicesStore::messages_cache_key(id),
                SaiCacheCleanLayer::Everywhere,
                cx,
            );
        }
    }

    pub fn restore_contact_join_dialogs_from_inbox(&mut self, cx: &mut Context<Self>) {
        let recent = self.recent_contact_join_events(cx);
        let allowed_virtual_ids: HashSet<String> =
            recent.iter().map(|event| Self::contact_join_virtual_chat_id(&event.id)).collect();
        let obsolete_virtual_ids: HashSet<String> = Self::chat_infos(cx)
            .into_iter()
            .filter(|chat| Self::is_contact_join_virtual_chat_id(&chat.id) && !allowed_virtual_ids.contains(&chat.id))
            .map(|chat| chat.id)
            .collect();
        if !obsolete_virtual_ids.is_empty() {
            Self::drop_virtual_chats(&obsolete_virtual_ids, cx);
        }
        for event in recent.iter().rev() {
            self.restore_contact_join_virtual_dialog(event, cx);
        }
    }

    pub fn clear_contact_join_local_data(&mut self, account_id: &str, cx: &mut Context<Self>) {
        if Self::contact_wire_user_id(cx).as_deref() != Some(account_id) {
            return;
        }
        SaiWsResponseCache::remove(
            &Self::contact_join_events_cache_key(account_id),
            SaiCacheCleanLayer::Everywhere,
            cx,
        );
        self.presented_contact_join_event_ids.clear();
        self.contact_join_visible_names.clear();
        cx.notify();

        let chats = Self::chat_infos(cx);
        let virtual_ids: HashSet<String> = chats
            .iter()
            .filter(|chat| Self::is_contact_join_virtual_chat_id(&chat.id))
            .map(|chat| chat.id.clone())
            .collect();
        if !virtual_ids.is_empty() {
            Self::drop_virtual_chats(&virtual_ids, cx);
        }

        let contact_joined = MessageSystemKind::ContactJoined.raw_value();
        for chat in chats.iter().filter(|chat| !virtual_ids.contains(&chat.id)) {
            let cache_key = MessageServicesStore::messages_cache_key(&chat.id);
            let payload = Self::contact_observed_payload(&cache_key, cx);
            let Some(root) = AnyDictionaryBridge::dictionary(payload.as_ref()) else {
                continue;
            };
            let Some(rows) = root.get(message_key::MESSAGES).and_then(Value::as_array) else {
                continue;
            };
            let rows: Vec<Map<String, Value>> = rows.iter().filter_map(|row| row.as_object().cloned()).collect();
            let row_count = rows.len();
            let filtered: Vec<Map<String, Value>> = rows
                .into_iter()
                .filter(|row| row.get(bot_key::SYSTEM_MESSAGE_TYPE).and_then(Value::as_str) != Some(contact_joined))
                .collect();
            if filtered.len() != row_count {
                let chat_id = chat.id.clone();
                MessageServicesStore::shared(cx)
                    .update(cx, |messages, cx| messages.patch_messages_root(&chat_id, filtered, cx));
                if let Some(updated) = Self::contact_current_payload(&cache_key, cx) {
                    let _ = SaiWsResponseCache::install_optimistic_root(
                        updated,
                        &cache_key,
                        true,
                        true,
                        SaiWsDiskCachePolicy::PreserveExisting,
                        false,
                        cx,
                    );
                }
            }
            if chat.last_message.as_ref().is_some_and(|message| message.id.starts_with("contact-join-event:")) {
                ChatServicesStore::shared(cx).update(cx, |chats, cx| {
                    chats.mutate_chat(
                        &chat.id,
                        |chat| {
                            chat.last_message = None;
                            if chat.unread_count == 1 {
                                chat.unread_count = 0;
                            }
                        },
                        cx,
                    );
                });
            }
        }
        ChatInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.refresh_chats(cx));
    }

    pub fn reoverlay_contact_join_events(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        let Some(chat) = Self::chat_infos(cx)
            .into_iter()
            .find(|chat| chat.id == chat_id && chat.r#type == ChatType::Direct)
        else {
            return;
        };
        for event in self.contact_join_events(cx) {
            let Some(contact) = event.contact.clone().filter(|contact| contact.id == chat.participant.id) else {
                continue;
            };
            self.install_contact_join_event(&event, &contact, chat_id, cx);
        }
    }

    pub fn apply_contact_join_events_read_optimistic(&mut self, event_ids: &[String], cx: &mut Context<Self>) {
        let Some(user_id) = Self::contact_wire_user_id(cx) else {
            return;
        };
        let key = Self::contact_join_events_cache_key(&user_id);
        let ids: HashSet<&String> = event_ids.iter().collect();
        let mut patch = Map::new();
        patch.insert(bot_key::IS_READ.to_owned(), Value::Bool(true));
        for id in &ids {
            let _ = SaiUpdater::patch_row(&key, user_key::EVENTS, user_key::ID, id, &patch, SaiUpdaterWriteScope::Cache, cx);
        }
        let local_ids: HashSet<String> = ids.iter().map(|id| Self::contact_join_local_message_id(id)).collect();
        for chat in Self::chat_infos(cx) {
            if !chat.last_message.as_ref().is_some_and(|message| local_ids.contains(&message.id)) {
                continue;
            }
            ChatServicesStore::shared(cx).update(cx, |chats, cx| {
                chats.mutate_chat(&chat.id, |chat| chat.unread_count = 0, cx);
            });
        }
        self.snapshot_contact_join_events_cache_in_memory(cx);
    }

    fn replace_contact_sync_settings(gateway: &UserContactSyncSettingsGatewayDTO, user_id: &str, cx: &mut App) {
        let Some(root) = SaiJSONCodec::encode(gateway) else {
            return;
        };
        Self::contact_replace_root(&Self::contact_sync_settings_cache_key(user_id), root, cx);
    }

    fn write_saved_contact_gateway(
        gateway: &UserSavedContactGatewayDTO,
        user_id: &str,
        target_user_id: &str,
        cx: &mut App,
    ) {
        let Some(root) = SaiJSONCodec::encode(gateway) else {
            return;
        };
        Self::contact_replace_root(&Self::saved_contact_cache_key(user_id, target_user_id), root, cx);
    }

    fn ensure_saved_contacts_list_root(user_id: &str, cx: &mut App) {
        let key = Self::saved_contacts_list_cache_key(user_id);
        if Self::contact_observed_payload(&key, cx).is_some() {
            return;
        }
        let root = UserSavedContactsListGatewayDTO {
            contacts: Vec::new(),
            has_more: false,
            relative_id: None,
            success: true,
            ..Default::default()
        };
        let Some(encoded) = SaiJSONCodec::encode(&root) else {
            return;
        };
        Self::contact_replace_root(&key, encoded, cx);
    }

    fn upsert_saved_contact_list(contact: &UserSavedContactWireDTO, user_id: &str, cx: &mut App) {
        Self::ensure_saved_contacts_list_root(user_id, cx);
        let key = Self::saved_contacts_list_cache_key(user_id);
        let Some(row) = SaiJSONCodec::encode(contact) else {
            return;
        };
        let _ = SaiUpdater::remove_row(
            &key,
            user_key::CONTACTS,
            user_key::TARGET_USER_ID,
            &contact.target_user_id,
            SaiUpdaterWriteScope::Cache,
            cx,
        );
        let _ = SaiUpdater::insert_row_if_missing(
            &key,
            user_key::CONTACTS,
            user_key::TARGET_USER_ID,
            row,
            true,
            SaiUpdaterWriteScope::Cache,
            cx,
        );
    }

    fn remove_saved_contact_from_list(target_user_id: &str, user_id: &str, cx: &mut App) {
        let _ = SaiUpdater::remove_row(
            &Self::saved_contacts_list_cache_key(user_id),
            user_key::CONTACTS,
            user_key::TARGET_USER_ID,
            target_user_id,
            SaiUpdaterWriteScope::Cache,
            cx,
        );
    }

    fn remember_contact_join_visible_name(&mut self, payload: Option<&Value>, contact_id: &str) {
        let Some(RiseEvent::ContactJoined(event)) = RiseEvent::decode_payload(name::CONTACT_JOINED, payload) else {
            return;
        };
        let name = trimming_whitespaces_and_newlines(&event.visible_name).to_owned();
        if name.is_empty() {
            return;
        }
        self.contact_join_visible_names.insert(contact_id.to_owned(), name);
    }

    fn decode_contact_join_event(payload: Option<&Value>) -> Option<UserContactJoinEventWireDTO> {
        match RiseEvent::decode_payload(name::CONTACT_JOINED, payload)? {
            RiseEvent::ContactJoined(event) => Some(UserContactJoinEventWireDTO::from_canon(&event)),
            _ => None,
        }
    }

    fn insert_contact_join_event(&mut self, event: &UserContactJoinEventWireDTO, cx: &mut Context<Self>) {
        let Some(user_id) = Self::contact_wire_user_id(cx) else {
            return;
        };
        let key = Self::contact_join_events_cache_key(&user_id);
        if Self::contact_observed_payload(&key, cx).is_none() {
            let empty = UserContactJoinEventsGatewayDTO {
                events: Vec::new(),
                has_more: false,
                relative_id: None,
                success: true,
                ..Default::default()
            };
            if let Some(root) = SaiJSONCodec::encode(&empty) {
                Self::contact_replace_root(&key, root, cx);
            }
        }
        if let Some(row) = SaiJSONCodec::encode(event) {
            let _ = SaiUpdater::insert_row_if_missing(
                &key,
                user_key::EVENTS,
                user_key::ID,
                row,
                true,
                SaiUpdaterWriteScope::Cache,
                cx,
            );
        }
        self.snapshot_contact_join_events_cache_in_memory(cx);
    }

    fn snapshot_contact_join_events_cache_in_memory(&mut self, cx: &mut Context<Self>) {
        let Some(user_id) = Self::contact_wire_user_id(cx) else {
            return;
        };
        let key = Self::contact_join_events_cache_key(&user_id);
        let Some(payload) = Self::contact_current_payload(&key, cx) else {
            return;
        };
        let _ = SaiWsResponseCache::install_optimistic_root(
            payload,
            &key,
            true,
            false,
            SaiWsDiskCachePolicy::PreserveExisting,
            false,
            cx,
        );
    }

    fn present_contact_join_event_if_needed(&mut self, event: &UserContactJoinEventWireDTO, cx: &mut Context<Self>) {
        if !self.presented_contact_join_event_ids.insert(event.id.clone()) {
            return;
        }
        if self.presented_contact_join_event_ids.len() > 512 {
            self.presented_contact_join_event_ids =
                self.presented_contact_join_event_ids.iter().take(256).cloned().collect();
        }
        cx.notify();
        let display_name = self.contact_join_display_name(event, cx);
        let shown_name = if display_name.is_empty() { tr("contact_unknown_name") } else { display_name.clone() };
        let text = tr_args("contact_joined_system_message", &[FormatArg::Text(Cow::Owned(shown_name))]);
        let contact = event.contact.as_ref();
        let action = contact.map(|contact| {
            NotifierAction::DeepLink(PushDeepLink::Chat {
                chat_id: Self::contact_join_chat_id(&contact.id, &event.id, cx),
            })
        });
        Notifier::send(
            text,
            NotifierSendOptions {
                r#type: NotifierType::Info,
                position: NotifierPosition::Top,
                icon: NotifierIcon::Avatar {
                    source: contact.map(|contact| contact.logo.clone()),
                    name: Some(display_name),
                    user_id: contact.map(|contact| contact.id.clone()),
                },
                action,
                duration: 5.2,
                is_notification: true,
                ..NotifierSendOptions::default()
            },
            cx,
        );
    }

    /// direct chat already open with this person, else the device-only dialog of the event
    fn contact_join_chat_id(contact_id: &str, event_id: &str, cx: &mut App) -> String {
        Self::chat_infos(cx)
            .into_iter()
            .find(|chat| {
                chat.r#type == ChatType::Direct
                    && !Self::is_contact_join_virtual_chat_id(&chat.id)
                    && chat.participant.id == contact_id
            })
            .map(|chat| chat.id)
            .unwrap_or_else(|| Self::contact_join_virtual_chat_id(event_id))
    }

    fn restore_contact_join_virtual_dialog(&mut self, event: &UserContactJoinEventWireDTO, cx: &mut Context<Self>) {
        let Some(contact) = event.contact.clone() else {
            return;
        };
        if AuthServicesStore::shared(cx).read(cx).is_current_user(&contact.id) {
            return;
        }
        let chat_id = Self::contact_join_chat_id(&contact.id, &event.id, cx);
        self.install_contact_join_event(event, &contact, &chat_id, cx);
        self.load_contact_join_alias_if_needed(&contact.id, cx);
    }

    fn load_contact_join_alias_if_needed(&mut self, target_user_id: &str, cx: &mut Context<Self>) {
        if target_user_id.is_empty()
            || self.saved_contact_gateway(target_user_id, cx).is_some()
            || self.contact_join_alias_requested_targets.contains(target_user_id)
            || UserActionsStore::slices(cx).get_saved_contact_sai.is_pending()
        {
            return;
        }
        self.contact_join_alias_requested_targets.insert(target_user_id.to_owned());
        let account_id = Self::contact_wire_user_id(cx);
        let this = cx.entity().downgrade();
        let target_user_id = target_user_id.to_owned();
        cx.defer(move |cx| {
            let request_target = target_user_id.clone();
            let on_result: UserContactCallback<anyhow::Result<UserSavedContactGatewayDTO>> =
                Box::new(move |result, cx: &mut App| {
                    if Self::contact_wire_user_id(cx) != account_id {
                        return;
                    }
                    let _ = this.update(cx, |services, cx| {
                        if result.is_err() {
                            services.contact_join_alias_requested_targets.remove(&target_user_id);
                            return;
                        }
                        let events: Vec<UserContactJoinEventWireDTO> = services
                            .contact_join_events(cx)
                            .into_iter()
                            .filter(|event| event.contact.as_ref().is_some_and(|contact| contact.id == target_user_id))
                            .collect();
                        for event in events {
                            services.restore_contact_join_virtual_dialog(&event, cx);
                        }
                    });
                });
            UserActionsStore::shared(cx)
                .update(cx, |actions, cx| actions.get_saved_contact_action(&request_target, Some(on_result), cx));
        });
    }

    pub fn migrate_contact_join_virtual_dialogs(&mut self, peer_id: &str, real_chat_id: &str, cx: &mut Context<Self>) {
        if peer_id.is_empty() || real_chat_id.is_empty() {
            return;
        }
        let chats = Self::chat_infos(cx);
        let virtual_chats: Vec<ChatInfo> = chats
            .iter()
            .filter(|chat| Self::is_contact_join_virtual_chat_id(&chat.id) && chat.participant.id == peer_id)
            .cloned()
            .collect();
        if !chats.iter().any(|chat| chat.id == real_chat_id)
            && let Some(mut fallback) = virtual_chats.first().cloned()
        {
            fallback.id = real_chat_id.to_owned();
            fallback.last_message = None;
            fallback.unread_count = 0;
            ChatServicesStore::shared(cx).update(cx, |chats, cx| chats.upsert_chat(fallback, cx));
        }
        for event in self.contact_join_events(cx) {
            let Some(contact) = event.contact.clone().filter(|contact| contact.id == peer_id) else {
                continue;
            };
            self.install_contact_join_event(&event, &contact, real_chat_id, cx);
        }
        if virtual_chats.is_empty() {
            return;
        }
        let virtual_ids: HashSet<String> = virtual_chats.into_iter().map(|chat| chat.id).collect();
        Self::drop_virtual_chats(&virtual_ids, cx);
    }

    fn install_contact_join_event(
        &mut self,
        event: &UserContactJoinEventWireDTO,
        contact: &UserContactUserWireDTO,
        chat_id: &str,
        cx: &mut Context<Self>,
    ) {
        let created_at: TimeInterval = event.created_at_ms as TimeInterval / 1_000.0;
        let display_name = self.contact_join_display_name(event, cx);
        let local_message_id = Self::contact_join_local_message_id(&event.id);
        let message = GetMessageMessage {
            id: local_message_id.clone(),
            chat_id: chat_id.to_owned(),
            sender_id: contact.id.clone(),
            sender_name: display_name.clone(),
            sender_tag: contact.tag.clone(),
            sender_logo: Some(contact.logo.clone()),
            content_type: MessageContentType::System,
            text: String::new(),
            created_at,
            system: Some(MessageSystemInfo {
                kind: MessageSystemKind::ContactJoined,
                actor_id: Some(contact.id.clone()),
                actor_name: Some(display_name.clone()),
                target_id: Some(contact.id.clone()),
                target_name: Some(display_name.clone()),
                text: None,
                ..Default::default()
            }),
            is_read: event.is_read,
            ..Default::default()
        };
        let participant = User {
            id: contact.id.clone(),
            name: display_name.clone(),
            tag: contact.tag.clone(),
            more: AuthorInfoMore { logo: contact.logo.clone(), is_premium: contact.is_premium, ..Default::default() },
            is_online: contact.is_online,
            last_seen: contact.last_seen,
            is_bot: contact.is_bot,
            ..Default::default()
        };

        let chat_exists = Self::chat_infos(cx).iter().any(|chat| chat.id == chat_id);
        let contact_user = contact.clone();
        let is_read = event.is_read;
        if chat_exists {
            let message = message.clone();
            ChatServicesStore::shared(cx).update(cx, |chats, cx| {
                chats.mutate_chat(
                    chat_id,
                    move |chat| {
                        let newer = chat.last_message.as_ref().is_none_or(|last| last.created_at <= created_at);
                        if !newer {
                            return;
                        }
                        chat.last_message = Some(message);
                        chat.updated_at = created_at;
                        if !is_read {
                            chat.unread_count = chat.unread_count.max(1);
                        }
                    },
                    cx,
                );
            });
        } else {
            let info = ChatInfo {
                id: chat_id.to_owned(),
                title: display_name.clone(),
                r#type: ChatType::Direct,
                last_message: Some(message),
                unread_count: if is_read { 0 } else { 1 },
                updated_at: created_at,
                created_at,
                participant,
                ..ChatInfo::default()
            };
            ChatServicesStore::shared(cx).update(cx, |chats, cx| chats.upsert_chat(info, cx));
        }

        if RiseConversationPresentation::shared(cx).read(cx).is_active() && !Self::is_contact_join_virtual_chat_id(chat_id)
        {
            Self::ingest_contact_join_into_engine(&local_message_id, chat_id, &contact_user, &display_name, created_at, cx);
        }

        let cache_key = MessageServicesStore::messages_cache_key(chat_id);
        let payload = Self::contact_observed_payload(&cache_key, cx);
        let root = AnyDictionaryBridge::dictionary(payload.as_ref()).unwrap_or_default();
        let mut rows = AnyDictionaryBridge::array_of_dictionaries(root.get(message_key::MESSAGES));
        if let Some(existing_index) =
            rows.iter().position(|row| row.get(bot_key::ID).and_then(Value::as_str) == Some(local_message_id.as_str()))
        {
            if rows[existing_index].get(bot_key::SENDER_NAME).and_then(Value::as_str) == Some(display_name.as_str()) {
                return;
            }
            rows.remove(existing_index);
        }
        let mut actor = Map::new();
        actor.insert(bot_key::ID.to_owned(), Value::String(contact.id.clone()));
        actor.insert(bot_key::NAME.to_owned(), Value::String(display_name.clone()));
        let mut row = Map::new();
        row.insert(bot_key::ID.to_owned(), Value::String(local_message_id));
        row.insert(bot_key::CHAT_ID.to_owned(), Value::String(chat_id.to_owned()));
        row.insert(bot_key::SENDER_ID.to_owned(), Value::String(contact.id.clone()));
        row.insert(bot_key::SENDER_NAME.to_owned(), Value::String(display_name));
        row.insert(bot_key::CONTENT.to_owned(), Value::String(String::new()));
        row.insert(bot_key::CONTENT_TYPE.to_owned(), Value::String(ChatMessageContentType::System.wire().to_owned()));
        row.insert(bot_key::CREATED_AT.to_owned(), Value::from(created_at));
        row.insert(bot_key::TIMESTAMP.to_owned(), Value::from(created_at));
        row.insert(bot_key::IS_READ.to_owned(), Value::Bool(is_read));
        row.insert(bot_key::IS_SYSTEM_MESSAGE.to_owned(), Value::Bool(true));
        row.insert(
            bot_key::SYSTEM_MESSAGE_TYPE.to_owned(),
            Value::String(MessageSystemKind::ContactJoined.raw_value().to_owned()),
        );
        row.insert(bot_key::SYSTEM_ACTOR_INFO.to_owned(), Value::Object(actor));
        rows.push(row);
        rows.sort_by(|lhs, rhs| {
            let lhs = lhs.get(bot_key::CREATED_AT).and_then(Value::as_f64).unwrap_or(0.0);
            let rhs = rhs.get(bot_key::CREATED_AT).and_then(Value::as_f64).unwrap_or(0.0);
            lhs.total_cmp(&rhs)
        });
        let chat_id = chat_id.to_owned();
        MessageServicesStore::shared(cx).update(cx, |messages, cx| messages.patch_messages_root(&chat_id, rows, cx));
    }

    fn ingest_contact_join_into_engine(
        local_message_id: &str,
        chat_id: &str,
        contact: &UserContactUserWireDTO,
        display_name: &str,
        created_at: TimeInterval,
        cx: &mut Context<Self>,
    ) {
        let mut actor = BTreeMap::new();
        actor.insert(bot_key::ID.to_owned(), RiseJSONValue::String(contact.id.clone()));
        actor.insert(bot_key::NAME.to_owned(), RiseJSONValue::String(display_name.to_owned()));
        let mut object: BTreeMap<String, RiseJSONValue> = BTreeMap::new();
        object.insert(bot_key::ID.to_owned(), RiseJSONValue::String(local_message_id.to_owned()));
        object.insert(bot_key::CHAT_ID.to_owned(), RiseJSONValue::String(chat_id.to_owned()));
        object.insert(bot_key::SENDER_ID.to_owned(), RiseJSONValue::String(contact.id.clone()));
        object.insert(bot_key::SENDER_NAME.to_owned(), RiseJSONValue::String(display_name.to_owned()));
        object.insert(bot_key::CONTENT.to_owned(), RiseJSONValue::String(String::new()));
        object.insert(
            bot_key::CONTENT_TYPE.to_owned(),
            RiseJSONValue::String(ChatMessageContentType::System.wire().to_owned()),
        );
        object.insert(bot_key::CREATED_AT.to_owned(), RiseJSONValue::Number(created_at));
        object.insert(bot_key::TIMESTAMP.to_owned(), RiseJSONValue::Number(created_at));
        object.insert(bot_key::IS_READ.to_owned(), RiseJSONValue::Bool(true));
        object.insert(bot_key::IS_SYSTEM_MESSAGE.to_owned(), RiseJSONValue::Bool(true));
        object.insert(
            bot_key::SYSTEM_MESSAGE_TYPE.to_owned(),
            RiseJSONValue::String(MessageSystemKind::ContactJoined.raw_value().to_owned()),
        );
        object.insert(bot_key::SYSTEM_ACTOR_INFO.to_owned(), RiseJSONValue::Object(actor));
        let Ok(wire) = RiseMessageWireCodec::wire(object) else {
            return;
        };
        let chat_id = chat_id.to_owned();
        cx.spawn(async move |_, cx: &mut AsyncApp| {
            let Ok(domain) = RiseConversationActionExecutor::require_domain(cx).await else {
                return;
            };
            domain.ingest_device_only_message(wire, &chat_id).await;
        })
        .detach();
    }

    fn contact_join_display_name(&self, event: &UserContactJoinEventWireDTO, cx: &mut App) -> String {
        let Some(contact) = event.contact.as_ref() else {
            return String::new();
        };
        let alias = self
            .saved_contact(&contact.id, cx)
            .map(|saved| trimming_whitespaces_and_newlines(&saved.resolved_display_name()).to_owned())
            .unwrap_or_default();
        if !alias.is_empty() {
            return alias;
        }
        if let Some(visible) = self.contact_join_visible_names.get(&contact.id) {
            return visible.clone();
        }
        trimming_whitespaces_and_newlines(&contact.name).to_owned()
    }
}
