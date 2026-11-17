use std::any::Any;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{App, Context, Entity};
use parking_lot::Mutex;
use rise_platform::process_activity::{ProcessActivityLease, begin_process_activity};
use rise_canon::rise_key::chat as chat_key;
use rise_canon::rise_key::message as message_key;
use rise_canon::{rise_proto, rise_request, rise_response, rise_wire_methods};
use rise_core::foundation::foundation_date::{Date, RiseClock, TimeInterval};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_errors::ErrorCode;
use rise_i18n::tr;
use serde_json::Value;

use crate::core::desktop_platform::rise_app_quit_drain::{QuitDrain, QuitLease};
use crate::core::lib::any_dictionary_bridge::AnyDictionaryBridge;
use crate::core::lib::canon_errors::CanonErrors;
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::core::utilities::get_sai_server_id::GetSaiServerId;
use crate::core::lib::sai::core::utilities::sai_cache::SaiCache;
use crate::core::lib::sai::core::utilities::sai_payload_path::SaiJSONCodec;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::{SaiSliceState, SaiTypedSlice, SaiWireEmptyDTO};
use crate::core::lib::sai::modules::sai_ws::sai_ws::{SaiWs, SaiWsOnError, SaiWsOnSuccess};
use crate::core::lib::sai::modules::sai_ws::sai_ws_optimistic_update::{
    SaiWsOptimisticAddStrategy, SaiWsOptimisticLocalFile, SaiWsOptimisticSuccessContext, SaiWsOptimisticUpdate,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws_outbound_cancellation::SaiWsOutboundCancelledError;
use crate::core::lib::sai::modules::sai_ws::sai_ws_pagination::{
    SaiWsPaginationCursorFallback, SaiWsPaginationDuplicatePolicy, SaiWsPaginationOptions,
    SaiWsPaginationRelativeIdArgs,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{
    SaiWsCachePreferredLayer, SaiWsRequestOptions, SaiWsRequestOptionsInit,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws_screen_refresh::SaiWsScreenRefresh;
use crate::core::lib::sai::sai::Sai;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::components::chat::composer::chat_draft_store::ChatDraftStore;
use crate::modules::chat::engine::core::rise_conversation_presentation::RiseConversationPresentation;
use crate::modules::chat::stores::chat_stores_hooks::chat_stores_user_hooks;
use crate::modules::chat::stores::chats::chat_actions::chat_archive_wire_types::{
    ChatArchiveMutationGatewayDTO, RiseStoredArchivedChatPage,
};
use crate::modules::chat::stores::chats::chat_actions::chat_folder_types::{
    CreateChatFolderResultDTO, EditChatFolderResultDTO, GetChatFoldersGatewayDTO,
};
use crate::modules::chat::stores::chats::chat_actions::chat_message_requests_wire_types::{
    DeleteAllMessageRequestsGatewayDTO, ResolveMessageRequestGatewayDTO, RiseStoredMessageRequestPage,
};
use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatInfo, ChatMuteScopes, ChatType, User};
use crate::modules::chat::stores::chats::chat_actions::chat_wire_types::{
    ChatAnalyticsGatewayDTO, ChatSettingsGatewayDTO, CreateChatResultDTO, CreateChatResultDTOExt, CreateInviteLinkResultDTO,
    CreateTopicResultDTO, EffectivePermissionsDTO, GetBlockedMembersGatewayDTO, GetChatActionsGatewayDTO,
    GetChatMembersGatewayDTO, GetChatPreviewGatewayDTO, GetInviteLinksGatewayDTO, GetJoinRequestsGatewayDTO,
    GetVoiceInviteLinkResultDTO,
    GetOverwritesGatewayDTO, JoinByLinkGatewayDTO, LinkedChatGatewayDTO, RequestJoinGatewayDTO, RiseStoredChat,
    RiseStoredChatDetail, RiseStoredChatPage, RiseStoredChatTopicList, RiseStoredForumStructure, RoleResultDTO,
    SectionResultDTO,
};
use crate::modules::chat::stores::chats::chat_interactions::chat_interactions::ChatInteractionsStore;
use crate::modules::chat::stores::chats::chat_services::chat_services::{ChatListSlices, ChatServicesStore};
use crate::modules::chat::widgets::bottomsheets::create_chat_sheet::create_chat_sheet_state::{
    ChatPrivacyType, CreateChatSheetState,
};
use crate::modules::user::stores::user::user_actions::user_wire_types::FriendListGatewayDTO;

pub type DeleteChatGatewayDTO = rise_proto::chat::ProtoDeleteChatResponse;

pub type ChatActionCallback = Rc<dyn Fn(&mut App)>;
pub type ChatActionTextCallback = Rc<dyn Fn(String, &mut App)>;
pub type ChatActionProgressCallback = Rc<dyn Fn(f64, &mut App)>;
pub type ChatActionResultCallback = Rc<dyn Fn(bool, Option<String>, &mut App)>;

const CHAT_DELETE_ACTIVITY: &str = "riseonly.chat.delete";

struct ChatDeleteBackgroundLease {
    transport_owner: String,
    holds_transport: Cell<bool>,
    activity: RefCell<Option<ProcessActivityLease>>,
    quit: RefCell<Option<QuitLease>>,
}

impl ChatDeleteBackgroundLease {
    fn new(cx: &mut App) -> Self {
        let transport_owner = format!("chat.delete.{}", Uuid::new_v4().uuid_string());
        SaiWs::set_transport_hold(&transport_owner, true, cx);
        Self {
            transport_owner,
            holds_transport: Cell::new(true),
            activity: RefCell::new(Some(begin_process_activity(CHAT_DELETE_ACTIVITY))),
            quit: RefCell::new(Some(QuitDrain::lease(CHAT_DELETE_ACTIVITY, cx))),
        }
    }

    fn end(&self, cx: &mut App) {
        self.activity.borrow_mut().take();
        self.quit.borrow_mut().take();
        if self.holds_transport.replace(false) {
            SaiWs::set_transport_hold(&self.transport_owner, false, cx);
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ChatDeletePendingSnapshot {
    pub chat_id: String,
    pub chats: Vec<ChatInfo>,
    pub archived: Option<ChatInfo>,
    pub chat: Option<ChatInfo>,
    pub was_in_chat_list: bool,
    pub clears_history: bool,
}

impl ChatDeletePendingSnapshot {
    pub fn is_official_bot_chat(&self) -> bool {
        self.chat.as_ref().is_some_and(ChatInfo::is_official_bot_chat)
    }

    pub fn keeps_chat(&self) -> bool {
        self.is_official_bot_chat() || self.clears_history
    }
}

pub struct ChatActionsStore {
    pub get_chats_sai: Rc<SaiTypedSlice<RiseStoredChatPage>>,
    pub get_archived_chats_sai: Rc<SaiTypedSlice<RiseStoredArchivedChatPage>>,
    pub get_common_chats_sai: Rc<SaiTypedSlice<RiseStoredChatPage>>,
    pub archive_chat_sai: Rc<SaiTypedSlice<ChatArchiveMutationGatewayDTO>>,
    pub unarchive_chat_sai: Rc<SaiTypedSlice<ChatArchiveMutationGatewayDTO>>,
    pub get_message_requests_sai: Rc<SaiTypedSlice<RiseStoredMessageRequestPage>>,
    pub resolve_message_request_sai: Rc<SaiTypedSlice<ResolveMessageRequestGatewayDTO>>,
    pub delete_all_message_requests_sai: Rc<SaiTypedSlice<DeleteAllMessageRequestsGatewayDTO>>,
    pub get_chat_by_id_sai: Rc<SaiTypedSlice<RiseStoredChatDetail>>,
    pub get_chat_members_sai: Rc<SaiTypedSlice<GetChatMembersGatewayDTO>>,
    pub ban_member_sai: Rc<SaiTypedSlice<SaiWireEmptyDTO>>,
    pub kick_member_sai: Rc<SaiTypedSlice<SaiWireEmptyDTO>>,
    pub add_member_sai: Rc<SaiTypedSlice<SaiWireEmptyDTO>>,
    pub edit_member_rules_sai: Rc<SaiTypedSlice<SaiWireEmptyDTO>>,
    pub edit_member_sai: Rc<SaiTypedSlice<SaiWireEmptyDTO>>,
    pub transfer_ownership_sai: Rc<SaiTypedSlice<SaiWireEmptyDTO>>,
    pub mute_chat_sai: Rc<SaiTypedSlice<SaiWireEmptyDTO>>,
    pub unmute_chat_sai: Rc<SaiTypedSlice<SaiWireEmptyDTO>>,
    pub update_chat_notifications_sai: Rc<SaiTypedSlice<SaiWireEmptyDTO>>,
    pub leave_chat_sai: Rc<SaiTypedSlice<SaiWireEmptyDTO>>,
    pub delete_chat_sai: Rc<SaiTypedSlice<DeleteChatGatewayDTO>>,
    pub unpin_chat_sai: Rc<SaiTypedSlice<SaiWireEmptyDTO>>,
    pub reorder_pinned_sai: Rc<SaiTypedSlice<SaiWireEmptyDTO>>,
    pub get_chat_folders_sai: Rc<SaiTypedSlice<GetChatFoldersGatewayDTO>>,
    pub folder_contacts_sai: Rc<SaiTypedSlice<FriendListGatewayDTO>>,
    pub folder_chat_search_slices: HashMap<String, Rc<SaiTypedSlice<RiseStoredChatPage>>>,
    pub folder_chat_search_cache_order: Vec<String>,
    pub create_chat_folder_sai: Rc<SaiTypedSlice<CreateChatFolderResultDTO>>,
    pub edit_chat_folder_sai: Rc<SaiTypedSlice<EditChatFolderResultDTO>>,
    pub delete_chat_folder_sai: Rc<SaiTypedSlice<SaiWireEmptyDTO>>,
    pub reorder_chat_folders_sai: Rc<SaiTypedSlice<SaiWireEmptyDTO>>,
    pub update_chat_sai: Rc<SaiTypedSlice<SaiWireEmptyDTO>>,
    pub update_chat_logo_sai: Rc<SaiTypedSlice<SaiWireEmptyDTO>>,
    pub set_auto_delete_sai: Rc<SaiTypedSlice<SaiWireEmptyDTO>>,
    pub create_chat_sai: Rc<SaiTypedSlice<CreateChatResultDTO>>,
    pub open_direct_chat_sai: Rc<SaiTypedSlice<CreateChatResultDTO>>,
    pub get_topics_sai: Rc<SaiTypedSlice<RiseStoredChatTopicList>>,
    pub create_topic_sai: Rc<SaiTypedSlice<CreateTopicResultDTO>>,
    pub delete_topic_sai: Rc<SaiTypedSlice<SaiWireEmptyDTO>>,
    pub update_topic_sai: Rc<SaiTypedSlice<rise_response::chat::UpdateTopic>>,
    pub update_topic_notifications_sai: Rc<SaiTypedSlice<rise_response::chat::UpdateTopicNotifications>>,
    pub set_topics_enabled_sai: Rc<SaiTypedSlice<SaiWireEmptyDTO>>,
    pub get_invite_links_sai: Rc<SaiTypedSlice<GetInviteLinksGatewayDTO>>,
    pub create_invite_link_sai: Rc<SaiTypedSlice<CreateInviteLinkResultDTO>>,
    pub get_voice_invite_link_sai: Rc<SaiTypedSlice<GetVoiceInviteLinkResultDTO>>,
    pub get_voice_invite_links_sai: Rc<SaiTypedSlice<GetInviteLinksGatewayDTO>>,
    pub revoke_voice_invite_link_sai: Rc<SaiTypedSlice<SaiWireEmptyDTO>>,
    pub revoke_invite_link_sai: Rc<SaiTypedSlice<SaiWireEmptyDTO>>,
    server_deleted_chat_ids: HashSet<String>,
    pub get_join_requests_sai: Rc<SaiTypedSlice<GetJoinRequestsGatewayDTO>>,
    pub get_chat_preview_sai: Rc<SaiTypedSlice<GetChatPreviewGatewayDTO>>,
    pub join_by_link_sai: Rc<SaiTypedSlice<JoinByLinkGatewayDTO>>,
    pub join_chat_sai: Rc<SaiTypedSlice<rise_response::chat::JoinChat>>,
    pub request_join_sai: Rc<SaiTypedSlice<RequestJoinGatewayDTO>>,
    pub approve_join_request_sai: Rc<SaiTypedSlice<SaiWireEmptyDTO>>,
    pub reject_join_request_sai: Rc<SaiTypedSlice<SaiWireEmptyDTO>>,
    pub admins_sai: Rc<SaiTypedSlice<GetChatMembersGatewayDTO>>,
    pub blocked_sai: Rc<SaiTypedSlice<GetBlockedMembersGatewayDTO>>,
    pub actions_sai: Rc<SaiTypedSlice<GetChatActionsGatewayDTO>>,
    pub unban_member_sai: Rc<SaiTypedSlice<SaiWireEmptyDTO>>,
    pub get_chat_settings_sai: Rc<SaiTypedSlice<ChatSettingsGatewayDTO>>,
    pub update_reactions_sai: Rc<SaiTypedSlice<ChatSettingsGatewayDTO>>,
    pub get_linked_chat_sai: Rc<SaiTypedSlice<LinkedChatGatewayDTO>>,
    pub link_chat_sai: Rc<SaiTypedSlice<SaiWireEmptyDTO>>,
    pub unlink_chat_sai: Rc<SaiTypedSlice<SaiWireEmptyDTO>>,
    pub chat_analytics_sai: Rc<SaiTypedSlice<ChatAnalyticsGatewayDTO>>,
    pub update_permissions_sai: Rc<SaiTypedSlice<ChatSettingsGatewayDTO>>,
    pub forum_structure_sai: Rc<SaiTypedSlice<RiseStoredForumStructure>>,
    pub eff_perms_sai: Rc<SaiTypedSlice<EffectivePermissionsDTO>>,
    pub create_role_sai: Rc<SaiTypedSlice<RoleResultDTO>>,
    pub update_role_sai: Rc<SaiTypedSlice<RoleResultDTO>>,
    pub delete_role_sai: Rc<SaiTypedSlice<SaiWireEmptyDTO>>,
    pub reorder_roles_sai: Rc<SaiTypedSlice<SaiWireEmptyDTO>>,
    pub assign_role_sai: Rc<SaiTypedSlice<SaiWireEmptyDTO>>,
    pub create_section_sai: Rc<SaiTypedSlice<SectionResultDTO>>,
    pub delete_section_sai: Rc<SaiTypedSlice<SaiWireEmptyDTO>>,
    pub update_section_sai: Rc<SaiTypedSlice<SectionResultDTO>>,
    pub reorder_sections_sai: Rc<SaiTypedSlice<SaiWireEmptyDTO>>,
    pub get_overwrites_sai: Rc<SaiTypedSlice<GetOverwritesGatewayDTO>>,
    pub set_overwrite_sai: Rc<SaiTypedSlice<SaiWireEmptyDTO>>,
    pub remove_overwrite_sai: Rc<SaiTypedSlice<SaiWireEmptyDTO>>,
    pub set_member_perms_sai: Rc<SaiTypedSlice<SaiWireEmptyDTO>>,
    pub member_override_sai: Rc<SaiTypedSlice<EffectivePermissionsDTO>>,
}

pub(crate) fn effective_user_id_for_wire(cx: &mut App) -> Option<String> {
    AuthServicesStore::shared(cx)
        .read(cx)
        .effective_user_id_for_wire()
        .filter(|user_id| !user_id.is_empty())
}

pub(crate) fn log_chat_failure(text: String, cx: &mut App) {
    Sai::log("Chat", move || text, SaiLogKind::Network, cx);
}

pub(crate) fn immediate_options(method: &str) -> SaiWsRequestOptions {
    SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
        service: rise_wire_methods::chat::SERVICE.to_owned(),
        method: method.to_owned(),
        reload_if_cached: true,
        allow_concurrent_while_pending: true,
        reflect_pending_in_ui: false,
        send_immediately: true,
        store_response_in_cache: false,
        ..SaiWsRequestOptionsInit::default()
    })
}

pub(crate) fn chat_page_pagination(relative_id: Option<String>) -> SaiWsPaginationOptions {
    SaiWsPaginationOptions::relative_id(SaiWsPaginationRelativeIdArgs {
        relative_id,
        response_relative_id_fallback: Some(SaiWsPaginationCursorFallback::LastItemId),
        has_more_path: Some(message_key::IS_HAVE_MORE.to_owned()),
        duplicate_policy: SaiWsPaginationDuplicatePolicy::UseIncoming,
        ..SaiWsPaginationRelativeIdArgs::default()
    })
}

fn logged_failure(prefix: String) -> SaiWsOnError {
    Rc::new(move |error: &anyhow::Error, cx: &mut App| {
        log_chat_failure(format!("{prefix} error: {error}"), cx);
    })
}

fn typed_payload<T: 'static>(payload: Option<Box<dyn Any>>) -> Option<T> {
    payload.and_then(|payload| payload.downcast::<T>().ok()).map(|typed| *typed)
}

impl ChatActionsStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(cx: &mut Context<Self>) -> Self {
        let slices = ChatListSlices::shared(cx);
        Self {
            get_chats_sai: slices.get_chats_sai,
            get_archived_chats_sai: slices.get_archived_chats_sai,
            get_common_chats_sai: SaiTypedSlice::new(cx),
            archive_chat_sai: SaiTypedSlice::new(cx),
            unarchive_chat_sai: SaiTypedSlice::new(cx),
            get_message_requests_sai: slices.get_message_requests_sai,
            resolve_message_request_sai: SaiTypedSlice::new(cx),
            delete_all_message_requests_sai: SaiTypedSlice::new(cx),
            get_chat_by_id_sai: slices.get_chat_by_id_sai,
            get_chat_members_sai: SaiTypedSlice::new(cx),
            ban_member_sai: SaiTypedSlice::new(cx),
            kick_member_sai: SaiTypedSlice::new(cx),
            add_member_sai: SaiTypedSlice::new(cx),
            edit_member_rules_sai: SaiTypedSlice::new(cx),
            edit_member_sai: SaiTypedSlice::new(cx),
            transfer_ownership_sai: SaiTypedSlice::new(cx),
            mute_chat_sai: SaiTypedSlice::new(cx),
            unmute_chat_sai: SaiTypedSlice::new(cx),
            update_chat_notifications_sai: SaiTypedSlice::new(cx),
            leave_chat_sai: SaiTypedSlice::new(cx),
            delete_chat_sai: SaiTypedSlice::new(cx),
            unpin_chat_sai: SaiTypedSlice::new(cx),
            reorder_pinned_sai: SaiTypedSlice::new(cx),
            get_chat_folders_sai: SaiTypedSlice::new(cx),
            folder_contacts_sai: SaiTypedSlice::new(cx),
            folder_chat_search_slices: HashMap::new(),
            folder_chat_search_cache_order: Vec::new(),
            create_chat_folder_sai: SaiTypedSlice::new(cx),
            edit_chat_folder_sai: SaiTypedSlice::new(cx),
            delete_chat_folder_sai: SaiTypedSlice::new(cx),
            reorder_chat_folders_sai: SaiTypedSlice::new(cx),
            update_chat_sai: SaiTypedSlice::new(cx),
            update_chat_logo_sai: SaiTypedSlice::new(cx),
            set_auto_delete_sai: SaiTypedSlice::new(cx),
            create_chat_sai: SaiTypedSlice::new(cx),
            open_direct_chat_sai: SaiTypedSlice::new(cx),
            get_topics_sai: SaiTypedSlice::new_unobserved(cx),
            create_topic_sai: SaiTypedSlice::new_unobserved(cx),
            delete_topic_sai: SaiTypedSlice::new_unobserved(cx),
            update_topic_sai: SaiTypedSlice::new_unobserved(cx),
            update_topic_notifications_sai: SaiTypedSlice::new_unobserved(cx),
            set_topics_enabled_sai: SaiTypedSlice::new_unobserved(cx),
            get_invite_links_sai: SaiTypedSlice::new(cx),
            create_invite_link_sai: SaiTypedSlice::new(cx),
            get_voice_invite_link_sai: SaiTypedSlice::new(cx),
            get_voice_invite_links_sai: SaiTypedSlice::new(cx),
            revoke_voice_invite_link_sai: SaiTypedSlice::new(cx),
            revoke_invite_link_sai: SaiTypedSlice::new(cx),
            server_deleted_chat_ids: HashSet::new(),
            get_join_requests_sai: SaiTypedSlice::new(cx),
            get_chat_preview_sai: SaiTypedSlice::new(cx),
            join_by_link_sai: SaiTypedSlice::new(cx),
            join_chat_sai: SaiTypedSlice::new(cx),
            request_join_sai: SaiTypedSlice::new(cx),
            approve_join_request_sai: SaiTypedSlice::new(cx),
            reject_join_request_sai: SaiTypedSlice::new(cx),
            admins_sai: SaiTypedSlice::new(cx),
            blocked_sai: SaiTypedSlice::new(cx),
            actions_sai: SaiTypedSlice::new(cx),
            unban_member_sai: SaiTypedSlice::new(cx),
            get_chat_settings_sai: SaiTypedSlice::new(cx),
            update_reactions_sai: SaiTypedSlice::new(cx),
            get_linked_chat_sai: SaiTypedSlice::new(cx),
            link_chat_sai: SaiTypedSlice::new(cx),
            unlink_chat_sai: SaiTypedSlice::new(cx),
            chat_analytics_sai: SaiTypedSlice::new(cx),
            update_permissions_sai: SaiTypedSlice::new(cx),
            forum_structure_sai: SaiTypedSlice::new_unobserved(cx),
            eff_perms_sai: SaiTypedSlice::new(cx),
            create_role_sai: SaiTypedSlice::new(cx),
            update_role_sai: SaiTypedSlice::new(cx),
            delete_role_sai: SaiTypedSlice::new(cx),
            reorder_roles_sai: SaiTypedSlice::new(cx),
            assign_role_sai: SaiTypedSlice::new(cx),
            create_section_sai: SaiTypedSlice::new_unobserved(cx),
            delete_section_sai: SaiTypedSlice::new_unobserved(cx),
            update_section_sai: SaiTypedSlice::new_unobserved(cx),
            reorder_sections_sai: SaiTypedSlice::new_unobserved(cx),
            get_overwrites_sai: SaiTypedSlice::new(cx),
            set_overwrite_sai: SaiTypedSlice::new(cx),
            remove_overwrite_sai: SaiTypedSlice::new(cx),
            set_member_perms_sai: SaiTypedSlice::new(cx),
            member_override_sai: SaiTypedSlice::new(cx),
        }
    }

    pub fn get_chats_action(
        &mut self,
        relative_id: Option<String>,
        up: bool,
        fetch_if_have_data: bool,
        reflect_pending_in_ui: Option<bool>,
        cx: &mut Context<Self>,
    ) {
        let Some(user_id) = effective_user_id_for_wire(cx) else {
            return;
        };

        let body = rise_request::chat::GetChats {
            limit: Some(ChatServicesStore::chats_limit()),
            q: None,
            relative_id: relative_id.clone(),
            up: Some(up),
        };

        let cache_key = ChatServicesStore::chats_cache_key_for_user(&user_id);
        let show_pending = reflect_pending_in_ui.unwrap_or_else(|| self.get_chats_sai.data().is_none());

        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::chat::SERVICE.to_owned(),
            method: rise_wire_methods::chat::GET_CHATS.to_owned(),
            cache_key: Some(cache_key),
            reload_if_cached: relative_id.is_some() || fetch_if_have_data,
            allow_concurrent_while_pending: false,
            reflect_pending_in_ui: show_pending,
            send_immediately: true,
            store_response_in_cache: true,
            persist_cache_to_disk: true,
            reload_when_only_disk_cached: false,
            preferred_cache_layer: SaiWsCachePreferredLayer::DiskFirst,
            path_to_array: Some(chat_key::CHATS.to_owned()),
            path_to_id: Some(chat_key::ID.to_owned()),
            pagination: Some(chat_page_pagination(relative_id)),
            screen_refresh: Some(SaiWsScreenRefresh::new(
                true,
                vec!["Chats".to_owned(), "ChatProfile".to_owned(), "Chat".to_owned()],
                None,
            )),
            ..SaiWsRequestOptionsInit::default()
        });

        let on_success: SaiWsOnSuccess = Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| {
            ChatServicesStore::shared(cx).update(cx, |services, cx| {
                services.normalize_chats_after_fetch(cx);
                services.reconcile_chat_archives_after_fetch(cx);
            });
            if let Some(hooks) = chat_stores_user_hooks() {
                (hooks.restore_contact_join_dialogs_from_inbox)(cx);
            }
        });
        let on_error: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            ChatServicesStore::shared(cx).update(cx, |services, cx| services.get_chats_error_handler(error, cx));
        });

        SaiWs::request_slice(&self.get_chats_sai, body, options, Some(on_success), Some(on_error), cx);
    }

    pub fn get_chat_by_id_action(&mut self, chat_id: &str, force: bool, cx: &mut Context<Self>) {
        if effective_user_id_for_wire(cx).is_none() {
            return;
        }

        let trimmed = trimming_whitespaces_and_newlines(chat_id).to_owned();
        let services = ChatServicesStore::shared(cx);

        if trimmed.is_empty()
            || GetSaiServerId::is_temp_id(Some(&trimmed))
            || (!force && services.read(cx).fetched_chat_by_id_set.contains(&trimmed))
        {
            return;
        }

        services.update(cx, |services, cx| {
            services.fetched_chat_by_id_set.insert(trimmed.clone());
            cx.notify();
        });

        let body = rise_request::chat::GetChatById { chat_id: trimmed.clone() };

        let success_id = trimmed.clone();
        let on_success: SaiWsOnSuccess = Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
            let dto = typed_payload::<RiseStoredChatDetail>(payload);
            let trimmed = success_id.clone();
            ChatServicesStore::shared(cx).update(cx, move |services, cx| {
                services.membership_refetch_in_flight.remove(&trimmed);
                cx.notify();
                let Some((wire, is_archived)) = dto
                    .filter(|dto| dto.success != Some(false))
                    .and_then(|dto| dto.chat.map(|wire| (wire, dto.is_archived)))
                else {
                    services.fetched_chat_by_id_set.remove(&trimmed);
                    services.community_preview_by_chat_id.remove(&trimmed);
                    return;
                };
                let info = wire.to_chat_info();
                if info.is_community_preview() || ChatServicesStore::is_discussion_guest_card(&info) {
                    services.community_preview_by_chat_id.insert(info.id.clone(), info);
                    services.fetched_chat_by_id_set.remove(&trimmed);
                    return;
                }
                services.community_preview_by_chat_id.remove(&info.id);
                if !ChatServicesStore::is_joined_member_role(info.role.as_deref()) {
                    return;
                }
                if services.is_archive_mutation_pending(&info.id) {
                    return;
                }
                match is_archived {
                    Some(true) => services.upsert_archived_chat(&info, true, cx),
                    Some(false) => {
                        services.remove_archived_chat_local(&info.id, cx);
                        services.upsert_chat(info, cx);
                    }
                    None => {
                        if services.contains_archived_chat(&info.id, cx) {
                            services.upsert_archived_chat(&info, false, cx);
                        } else if services.archived_chat_membership_is_fully_resolved(cx)
                            || services.current_chat_infos(cx).iter().any(|chat| services.canonical_chat_id(&chat.id) == info.id)
                        {
                            services.upsert_chat(info, cx);
                        }
                    }
                }
            });
        });
        let on_error: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            ChatServicesStore::shared(cx).update(cx, |services, cx| {
                services.membership_refetch_in_flight.remove(&trimmed);
                services.fetched_chat_by_id_set.remove(&trimmed);
                services.community_preview_by_chat_id.remove(&trimmed);
                cx.notify();
            });
            log_chat_failure(format!("get_chat_by_id({trimmed}) error: {error}"), cx);
        });

        SaiWs::request_slice(
            &self.get_chat_by_id_sai,
            body,
            immediate_options(rise_wire_methods::chat::GET_CHAT_BY_ID),
            Some(on_success),
            Some(on_error),
            cx,
        );
    }

    pub fn mute_chat_action(
        &mut self,
        chat_id: &str,
        mute_until_sec: Option<TimeInterval>,
        scopes: Option<ChatMuteScopes>,
        cx: &mut Context<Self>,
    ) {
        if effective_user_id_for_wire(cx).is_none() {
            return;
        }

        ChatServicesStore::shared(cx)
            .update(cx, |services, cx| services.mute_chat_local(chat_id, true, mute_until_sec, cx));

        let mut body = rise_request::chat::MuteChat::new(chat_id.to_owned());
        if let Some(until) = mute_until_sec {
            body.mute_until = Some(until as i64);
        }
        if let Some(scopes) = scopes {
            body.mute_messages = Some(scopes.messages);
            body.mute_mentions = Some(scopes.mentions);
            body.mute_replies = Some(scopes.replies);
            body.mute_reactions = Some(scopes.reactions);
            body.mute_events = Some(scopes.events);
        }

        let failed_id = chat_id.to_owned();
        let on_error: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            log_chat_failure(format!("mute_chat({failed_id}) error: {error}"), cx);
            ChatServicesStore::shared(cx)
                .update(cx, |services, cx| services.mute_chat_local(&failed_id, false, None, cx));
        });

        SaiWs::request_slice(
            &self.mute_chat_sai,
            body,
            immediate_options(rise_wire_methods::chat::MUTE_CHAT),
            None,
            Some(on_error),
            cx,
        );
    }

    pub fn unmute_chat_action(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        if effective_user_id_for_wire(cx).is_none() {
            return;
        }

        ChatServicesStore::shared(cx).update(cx, |services, cx| services.mute_chat_local(chat_id, false, None, cx));

        let body = rise_request::chat::UnmuteChat { chat_id: chat_id.to_owned() };

        SaiWs::request_slice(
            &self.unmute_chat_sai,
            body,
            immediate_options(rise_wire_methods::chat::UNMUTE_CHAT),
            None,
            Some(logged_failure(format!("unmute_chat({chat_id})"))),
            cx,
        );
    }

    pub fn update_chat_notifications_action(
        &mut self,
        chat_id: &str,
        scopes: ChatMuteScopes,
        on_success: ChatActionCallback,
        on_error: ChatActionCallback,
        cx: &mut Context<Self>,
    ) {
        if effective_user_id_for_wire(cx).is_none() {
            cx.defer(move |cx| on_error(cx));
            return;
        }

        let mut body = rise_request::chat::UpdateChatNotifications::new(chat_id.to_owned());
        body.events = Some(scopes.events);
        body.mentions = Some(scopes.mentions);
        body.messages = Some(scopes.messages);
        body.reactions = Some(scopes.reactions);
        body.replies = Some(scopes.replies);

        let on_success: SaiWsOnSuccess = Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| on_success(cx));
        let failed_id = chat_id.to_owned();
        let on_failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            log_chat_failure(format!("update_chat_notifications({failed_id}) error: {error}"), cx);
            on_error(cx);
        });

        SaiWs::request_slice(
            &self.update_chat_notifications_sai,
            body,
            immediate_options(rise_wire_methods::chat::UPDATE_CHAT_NOTIFICATIONS),
            Some(on_success),
            Some(on_failure),
            cx,
        );
    }

    pub fn leave_chat_action(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        if effective_user_id_for_wire(cx).is_none() {
            return;
        }

        let services = ChatServicesStore::shared(cx);
        services.update(cx, |services, cx| services.remove_chat_local(chat_id, cx));
        ChatDraftStore::shared(cx).update(cx, |drafts, cx| drafts.clear_chat(chat_id, cx));
        let removed_archived = services.update(cx, |services, cx| services.remove_archived_chat_local(chat_id, cx));

        let body = rise_request::chat::LeaveChat { chat_id: chat_id.to_owned() };

        let on_success: SaiWsOnSuccess = Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| {
            if removed_archived {
                ChatInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.refresh_archived_chats(cx));
            }
        });

        SaiWs::request_slice(
            &self.leave_chat_sai,
            body,
            immediate_options(rise_wire_methods::chat::LEAVE_CHAT),
            Some(on_success),
            Some(logged_failure(format!("leave_chat({chat_id})"))),
            cx,
        );
    }

    pub fn begin_chat_delete_action(
        &mut self,
        chat_id: &str,
        known_chat: Option<ChatInfo>,
        clear_history: bool,
        cx: &mut Context<Self>,
    ) -> ChatDeletePendingSnapshot {
        let services = ChatServicesStore::shared(cx);
        let chats = services.update(cx, |services, cx| services.current_chat_infos(cx));
        let archived = services
            .update(cx, |services, cx| services.archived_chats(cx))
            .into_iter()
            .find(|chat| chat.id == chat_id);
        let listed = chats.iter().find(|chat| chat.id == chat_id).cloned();
        let was_in_chat_list = listed.is_some();
        let chat = listed
            .or_else(|| archived.clone())
            .or_else(|| RiseConversationPresentation::shared(cx).read(cx).chat(chat_id).cloned())
            .or_else(|| known_chat.filter(|known| known.id == chat_id));

        let pending = ChatDeletePendingSnapshot {
            chat_id: chat_id.to_owned(),
            chats,
            archived,
            chat,
            was_in_chat_list,
            clears_history: clear_history,
        };

        if !pending.keeps_chat() {
            services.update(cx, |services, cx| {
                services.pending_delete_chat_ids.insert(chat_id.to_owned());
                cx.notify();
                services.remove_chat_local(chat_id, cx);
                services.remove_archived_chat_local(chat_id, cx);
            });
        }

        pending
    }

    pub fn cancel_chat_delete_action(&mut self, pending: &ChatDeletePendingSnapshot, cx: &mut Context<Self>) {
        if pending.keeps_chat() {
            return;
        }

        ChatServicesStore::shared(cx).update(cx, |services, cx| {
            if let Some(chat) = pending.chat.clone().filter(|_| pending.was_in_chat_list) {
                services.restore_chat_after_pending_delete(chat, cx);
            } else {
                services.clear_pending_delete_marks(&pending.chat_id, cx);
            }
            if let Some(archived) = pending.archived.clone() {
                services.restore_archived_chat_local(&archived, cx);
            }
        });
    }

    pub fn rollback_chat_delete_action(&mut self, pending: &ChatDeletePendingSnapshot, cx: &mut Context<Self>) {
        if self.is_delete_confirmed(&pending.chat_id, cx) {
            return;
        }

        self.cancel_chat_delete_action(pending, cx);
    }

    pub fn delete_chat_action(
        &mut self,
        chat_id: &str,
        delete_for_both: bool,
        clear_history: bool,
        pending: Option<ChatDeletePendingSnapshot>,
        on_error: Option<ChatActionCallback>,
        cx: &mut Context<Self>,
    ) {
        if effective_user_id_for_wire(cx).is_none() {
            if let Some(on_error) = on_error {
                cx.defer(move |cx| on_error(cx));
            }
            return;
        }

        let pending = match pending {
            Some(pending) => pending,
            None => self.begin_chat_delete_action(chat_id, None, clear_history, cx),
        };
        let keeps_chat = pending.keeps_chat();

        let mut body = rise_request::chat::DeleteChat::new(chat_id.to_owned());
        body.clear_history = pending.clears_history.then_some(true);
        body.delete_for_both = Some(delete_for_both);

        let lease = Rc::new(ChatDeleteBackgroundLease::new(cx));
        let pending = Rc::new(pending);

        let success_id = chat_id.to_owned();
        let success_pending = pending.clone();
        let success_lease = lease.clone();
        let success_error = on_error.clone();
        let on_success: SaiWsOnSuccess = Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
            let chat_id = success_id.as_str();
            let pending = success_pending.clone();
            let services = ChatServicesStore::shared(cx);
            services.update(cx, |services, cx| {
                services.pending_delete_chat_ids.remove(chat_id);
                cx.notify();
            });
            let notify_error = |cx: &mut App| {
                if let Some(on_error) = &success_error {
                    on_error(cx);
                }
            };
            match typed_payload::<DeleteChatGatewayDTO>(payload) {
                None => {
                    log_chat_failure(format!("delete_chat({chat_id}) returned an invalid response"), cx);
                    if keeps_chat {
                        services.update(cx, |services, cx| services.preserve_official_bot_after_history_clear(chat_id, cx));
                        notify_error(cx);
                    } else {
                        let on_error = success_error.clone();
                        ChatActionsStore::shared(cx).update(cx, |actions, cx| {
                            actions.settle_delete_failure((*pending).clone(), on_error, cx)
                        });
                    }
                }
                Some(dto) if !dto.success => {
                    let reason = if dto.error.is_empty() { "unknown server error" } else { dto.error.as_str() };
                    log_chat_failure(format!("delete_chat({chat_id}) rejected: {reason}"), cx);
                    let on_error = success_error.clone();
                    ChatActionsStore::shared(cx)
                        .update(cx, |actions, cx| actions.settle_delete_failure((*pending).clone(), on_error, cx));
                }
                Some(_) if keeps_chat => {
                    services.update(cx, |services, cx| services.preserve_official_bot_after_history_clear(chat_id, cx));
                }
                Some(_) => {
                    services.update(cx, |services, cx| {
                        services.confirmed_removed_chat_ids.insert(chat_id.to_owned());
                        cx.notify();
                    });
                    ChatDraftStore::shared(cx).update(cx, |drafts, cx| drafts.clear_chat(chat_id, cx));
                    ChatActionsStore::shared(cx).update(cx, |actions, cx| {
                        actions.discard_peer_draft_chat(chat_id, pending.chat.as_ref(), cx);
                    });
                    if pending.archived.is_some() {
                        ChatInteractionsStore::shared(cx)
                            .update(cx, |interactions, cx| interactions.refresh_archived_chats(cx));
                    }
                }
            }
            success_lease.end(cx);
        });

        let failed_id = chat_id.to_owned();
        let on_failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            ChatServicesStore::shared(cx).update(cx, |services, cx| {
                services.pending_delete_chat_ids.remove(&failed_id);
                cx.notify();
            });
            log_chat_failure(format!("delete_chat({failed_id}) error: {error}"), cx);
            let on_error = on_error.clone();
            ChatActionsStore::shared(cx)
                .update(cx, |actions, cx| actions.settle_delete_failure((*pending).clone(), on_error, cx));
            lease.end(cx);
        });

        SaiWs::request_slice(
            &self.delete_chat_sai,
            body,
            immediate_options(rise_wire_methods::chat::DELETE_CHAT),
            Some(on_success),
            Some(on_failure),
            cx,
        );
    }

    const DELETE_CONFIRM_GRACE: Duration = Duration::from_secs(2);
    const DELETE_CONFIRM_POLL: Duration = Duration::from_millis(100);

    /// a failed answer doesnt prove the chat survived, dissolving an owned group tells members first
    /// so the answer can come late or refuse a repeat while the chat is gone
    /// the own chat_deleted event settles it
    fn settle_delete_failure(
        &mut self,
        pending: ChatDeletePendingSnapshot,
        on_error: Option<ChatActionCallback>,
        cx: &mut Context<Self>,
    ) {
        let chat_id = pending.chat_id.clone();
        cx.spawn(async move |this, cx| {
            let deadline = RiseClock::monotonic() + Self::DELETE_CONFIRM_GRACE;
            loop {
                let Ok(confirmed) = this.update(cx, |this, cx| this.is_delete_confirmed(&chat_id, cx)) else {
                    return;
                };
                if confirmed || RiseClock::monotonic() >= deadline {
                    break;
                }
                cx.background_executor().timer(Self::DELETE_CONFIRM_POLL).await;
            }
            let _ = this.update(cx, |this, cx| {
                if this.is_delete_confirmed(&chat_id, cx) {
                    this.server_deleted_chat_ids.remove(&chat_id);
                    return;
                }
                this.rollback_chat_delete_action(&pending, cx);
                if let Some(on_error) = on_error {
                    cx.defer(move |cx| on_error(cx));
                }
            });
        })
        .detach();
    }

    pub fn note_server_deleted_chat(&mut self, chat_id: &str) {
        if chat_id.is_empty() {
            return;
        }
        self.server_deleted_chat_ids.insert(chat_id.to_owned());
    }

    fn is_delete_confirmed(&self, chat_id: &str, cx: &mut App) -> bool {
        self.server_deleted_chat_ids.contains(chat_id)
            || ChatServicesStore::shared(cx).read(cx).confirmed_removed_chat_ids.contains(chat_id)
    }

    pub fn discard_peer_draft_chat(&mut self, after_deleting: &str, chat: Option<&ChatInfo>, cx: &mut Context<Self>) {
        let services = ChatServicesStore::shared(cx);
        let mut draft_ids: HashSet<String> = services
            .read(cx)
            .temp_chat_id_to_real
            .iter()
            .filter(|(draft_id, real_id)| {
                real_id.as_str() == after_deleting && ChatServicesStore::is_draft_direct_chat_id(draft_id)
            })
            .map(|(draft_id, _)| draft_id.clone())
            .collect();
        if let Some(chat) = chat
            && chat.r#type == ChatType::Direct
        {
            let draft_id = ChatServicesStore::draft_direct_chat_id(&chat.participant.id);
            if !draft_id.is_empty() {
                draft_ids.insert(draft_id);
            }
        }
        for draft_id in draft_ids {
            services.update(cx, |services, cx| {
                services.temp_chat_id_to_real.remove(&draft_id);
                cx.notify();
            });
            ChatInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                interactions.delete_chat(&draft_id, false, false, None, None, None, cx);
            });
        }
    }

    pub fn unpin_chat_action(
        &mut self,
        chat_id: &str,
        folder_id: Option<&str>,
        on_error: ChatActionCallback,
        cx: &mut Context<Self>,
    ) {
        if effective_user_id_for_wire(cx).is_none() || chat_id.is_empty() {
            return;
        }

        let failed_id = chat_id.to_owned();
        let on_failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            log_chat_failure(format!("unpin_chat({failed_id}) error: {error}"), cx);
            on_error(cx);
        });

        SaiWs::request_slice(
            &self.unpin_chat_sai,
            rise_request::chat::UnpinChat { chat_id: chat_id.to_owned(), folder_id: folder_id.map(str::to_owned) },
            immediate_options(rise_wire_methods::chat::UNPIN_CHAT),
            None,
            Some(on_failure),
            cx,
        );
    }

    pub fn reorder_pinned_chats_action(
        &mut self,
        chat_ids: Vec<String>,
        folder_id: Option<&str>,
        on_error: SaiWsOnError,
        cx: &mut Context<Self>,
    ) {
        if effective_user_id_for_wire(cx).is_none() {
            return;
        }

        let on_failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            log_chat_failure(format!("reorder_pinned_chats error: {error}"), cx);
            on_error(error, cx);
        });

        SaiWs::request_slice(
            &self.reorder_pinned_sai,
            rise_request::chat::ReorderPinnedChats { chat_ids: Some(chat_ids), folder_id: folder_id.map(str::to_owned) },
            immediate_options(rise_wire_methods::chat::REORDER_PINNED_CHATS),
            None,
            Some(on_failure),
            cx,
        );
    }

    pub fn update_chat_action(
        &mut self,
        chat_id: &str,
        title: Option<String>,
        description: Option<String>,
        logo_url: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if effective_user_id_for_wire(cx).is_none() {
            return;
        }

        let mut body = rise_request::chat::UpdateChat::new(chat_id.to_owned());
        body.title = title.clone();
        body.description = description.clone();
        body.logo_url = logo_url.clone();

        let services = ChatServicesStore::shared(cx);
        let previous = services.read(cx).chat_info(chat_id, cx);
        services.update(cx, |services, cx| {
            services.update_chat_local(chat_id, title.as_deref(), description.as_deref(), logo_url.as_deref(), cx);
        });

        let failed_id = chat_id.to_owned();
        let on_error: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            log_chat_failure(format!("update_chat({failed_id}) error: {error}"), cx);
            if let Some(previous) = &previous {
                let services = ChatServicesStore::shared(cx);
                let current = services.read(cx).chat_info(&failed_id, cx);
                let rollback_title = (title.is_some()
                    && current.as_ref().map(|chat| chat.title.as_str()) == title.as_deref())
                .then(|| previous.title.as_str());
                let rollback_description = (description.is_some()
                    && current.as_ref().and_then(|chat| chat.description.as_deref()) == description.as_deref())
                .then(|| previous.description.as_deref().unwrap_or_default());
                let rollback_logo = (logo_url.is_some()
                    && current.as_ref().and_then(|chat| chat.logo_url.as_deref()) == logo_url.as_deref())
                .then(|| previous.logo_url.as_deref().unwrap_or_default());
                services.update(cx, |services, cx| {
                    services.update_chat_local(&failed_id, rollback_title, rollback_description, rollback_logo, cx);
                });
            }
            Self::report_member_mutation_failure(error, cx);
        });

        SaiWs::request_slice(
            &self.update_chat_sai,
            body,
            immediate_options(rise_wire_methods::chat::UPDATE_CHAT),
            None,
            Some(on_error),
            cx,
        );
    }

    pub fn update_chat_visibility_action(
        &mut self,
        chat_id: &str,
        is_public: Option<bool>,
        tag: &str,
        forward_in_chat_enabled: Option<bool>,
        on_result: ChatActionResultCallback,
        cx: &mut Context<Self>,
    ) {
        if effective_user_id_for_wire(cx).is_none() || chat_id.is_empty() {
            cx.defer(move |cx| on_result(false, None, cx));
            return;
        }

        let mut body = rise_request::chat::UpdateChat::new(chat_id.to_owned());
        body.forward_in_chat_enabled = forward_in_chat_enabled;
        body.is_public = is_public;
        body.tag = is_public.map(|public| {
            if public { trimming_whitespaces_and_newlines(tag).to_owned() } else { String::new() }
        });

        let refetch_id = chat_id.to_owned();
        let success_result = on_result.clone();
        let on_success: SaiWsOnSuccess = Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| {
            ChatActionsStore::shared(cx).update(cx, |actions, cx| actions.get_chat_by_id_action(&refetch_id, true, cx));
            success_result(true, None, cx);
        });
        let failed_id = chat_id.to_owned();
        let on_error: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            log_chat_failure(format!("update_chat visibility({failed_id}) error: {error}"), cx);
            let refusal = CanonErrors::failure(error).and_then(|failure| CanonErrors::text_for(&failure));
            on_result(false, refusal, cx);
        });

        SaiWs::request_slice(
            &self.update_chat_sai,
            body,
            immediate_options(rise_wire_methods::chat::UPDATE_CHAT),
            Some(on_success),
            Some(on_error),
            cx,
        );
    }

    pub fn update_chat_logo_action(
        &mut self,
        chat_id: &str,
        local_file: SaiWsOptimisticLocalFile,
        on_progress: ChatActionProgressCallback,
        on_error: ChatActionCallback,
        cx: &mut Context<Self>,
    ) {
        if effective_user_id_for_wire(cx).is_none() || chat_id.is_empty() {
            return;
        }

        let file_error = on_error.clone();
        let slice = self.update_chat_logo_sai.clone();
        let optimistic = SaiWsOptimisticUpdate {
            enabled: true,
            files: Some(ChatServicesStore::make_logo_optimistic_files(
                local_file,
                on_progress.clone(),
                Rc::new(move |_: &anyhow::Error, cx: &mut App| file_error(cx)),
                Rc::new(move |error: &anyhow::Error, _: &mut App| slice.mark_rejected(error)),
            )),
            ..SaiWsOptimisticUpdate::default()
        };

        let body = rise_request::chat::UpdateChat::new(chat_id.to_owned());

        let mut options = immediate_options(rise_wire_methods::chat::UPDATE_CHAT);
        options.optimistic_update = Some(optimistic);

        let on_success: SaiWsOnSuccess = Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| on_progress(100.0, cx));
        let failed_id = chat_id.to_owned();
        let on_failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            on_error(cx);
            log_chat_failure(format!("update_chat(logo)({failed_id}) error: {error}"), cx);
            if error.is::<SaiWsOutboundCancelledError>() {
                return;
            }
            Self::report_member_mutation_failure(error, cx);
        });

        SaiWs::request_slice(&self.update_chat_logo_sai, body, options, Some(on_success), Some(on_failure), cx);
    }

    pub fn set_auto_delete_action(&mut self, chat_id: &str, seconds: i64, cx: &mut Context<Self>) {
        if effective_user_id_for_wire(cx).is_none() || chat_id.is_empty() {
            return;
        }

        let mut body = rise_request::chat::UpdateChat::new(chat_id.to_owned());
        body.auto_delete_messages_after = Some(seconds);

        let failed_id = chat_id.to_owned();
        let on_error: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            log_chat_failure(format!("set_auto_delete({failed_id}) error: {error}"), cx);
            Self::report_member_mutation_failure(error, cx);
        });

        SaiWs::request_slice(
            &self.set_auto_delete_sai,
            body,
            immediate_options(rise_wire_methods::chat::UPDATE_CHAT),
            None,
            Some(on_error),
            cx,
        );
    }

    pub fn create_group_action(
        &mut self,
        local_logo_file: Option<SaiWsOptimisticLocalFile>,
        on_success: Option<ChatActionTextCallback>,
        on_error: Option<ChatActionTextCallback>,
        cx: &mut Context<Self>,
    ) {
        let sheet = CreateChatSheetState::shared(cx);
        let state = sheet.read(cx);
        let is_public = state.group_privacy == ChatPrivacyType::PublicChat;
        let title = state.group_title().to_owned();
        let description = state.group_description.clone();
        let tag = is_public.then(|| state.group_tag.clone());
        let forward_enabled = !state.forward_in_group_blocked;
        let member_ids = state.group_members.iter().map(|member| member.id.clone()).collect();
        let local_logo_file = local_logo_file.or_else(|| state.group_local_logo_file.clone());
        tracing::warn!(target: "riseonly::X844", has_logo = local_logo_file.is_some(), "create group with logo");
        self.create_chat_action(
            "GROUP",
            title,
            description,
            tag,
            is_public,
            forward_enabled,
            member_ids,
            local_logo_file,
            on_success,
            on_error,
            cx,
        );
    }

    pub fn create_channel_action(
        &mut self,
        local_logo_file: Option<SaiWsOptimisticLocalFile>,
        on_success: Option<ChatActionTextCallback>,
        on_error: Option<ChatActionTextCallback>,
        cx: &mut Context<Self>,
    ) {
        let sheet = CreateChatSheetState::shared(cx);
        let state = sheet.read(cx);
        let is_public = state.channel_privacy == ChatPrivacyType::PublicChat;
        let title = state.channel_title.clone();
        let description = state.channel_description.clone();
        let tag = is_public.then(|| state.channel_tag.clone());
        let forward_enabled = !state.forward_in_channel_blocked;
        let local_logo_file = local_logo_file.or_else(|| state.channel_local_logo_file.clone());
        self.create_chat_action(
            "CHANNEL",
            title,
            description,
            tag,
            is_public,
            forward_enabled,
            Vec::new(),
            local_logo_file,
            on_success,
            on_error,
            cx,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn create_chat_action(
        &mut self,
        r#type: &str,
        title: String,
        description: String,
        tag: Option<String>,
        is_public: bool,
        forward_enabled: bool,
        member_ids: Vec<String>,
        local_logo_file: Option<SaiWsOptimisticLocalFile>,
        on_success: Option<ChatActionTextCallback>,
        on_error: Option<ChatActionTextCallback>,
        cx: &mut Context<Self>,
    ) {
        let sheet = CreateChatSheetState::shared(cx);

        let Some(user_id) = effective_user_id_for_wire(cx) else {
            Self::abandon_create_chat(&sheet, cx);
            return;
        };

        let trimmed_title = trimming_whitespaces_and_newlines(&title).to_owned();
        let normalized_tag = trimming_whitespaces_and_newlines(tag.as_deref().unwrap_or_default()).to_lowercase();

        if trimmed_title.is_empty() || (r#type == "CHANNEL" && is_public && normalized_tag.is_empty()) {
            Self::abandon_create_chat(&sheet, cx);
            return;
        }

        let client_request_id = {
            let state = sheet.read(cx);
            let request_id = if r#type == "CHANNEL" {
                state.channel_create_request_id()
            } else {
                state.group_create_request_id()
            };
            request_id.to_owned()
        };
        let mut participants = vec![user_id.clone()];
        participants.extend(member_ids.into_iter().filter(|member_id| *member_id != user_id));

        let mut request = rise_request::chat::CreateChat::new(trimmed_title, r#type.to_owned());
        request.banner_url = Some(String::new());
        request.client_request_id = Some(client_request_id);
        request.description = Some(trimming_whitespaces_and_newlines(&description).to_owned());
        request.forward_in_chat_enabled = Some(forward_enabled);
        request.is_public = Some(is_public);
        request.logo_url = Some(String::new());
        request.participants = Some(participants);
        request.tag = (!normalized_tag.is_empty()).then_some(normalized_tag);
        let body = SaiJSONCodec::encode(&request).unwrap_or_default();

        let cache_key = SaiCache::new(["getChats", user_id.as_str()]).key;

        let coordinator_temp_id: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let local_logo_url = local_logo_file.as_ref().map(|file| file.url.to_string());
        let added_temp_id = coordinator_temp_id.clone();
        let mut optimistic = SaiWsOptimisticUpdate {
            enabled: true,
            add_strategy: SaiWsOptimisticAddStrategy::Start,
            create_temp_data: Some(Arc::new(move |outbound: &Value, cx: &mut App| -> anyhow::Result<Value> {
                let dict = AnyDictionaryBridge::dictionary(Some(outbound)).unwrap_or_else(|| body.clone());
                let temp_id = format!("temp_create_chat_{}", (Date::now().time_interval_since_1970() * 1000.0) as i64);
                Ok(Value::Object(ChatServicesStore::make_create_chat_temp_row(
                    &dict,
                    &temp_id,
                    local_logo_url.as_deref(),
                    cx,
                )))
            })),
            extract_real_data: Some(Arc::new(move |any: &Value, _: &SaiWsOptimisticSuccessContext, _: &mut App| -> anyhow::Result<Value> {
                Ok(ChatServicesStore::extract_real_chat_row(any).map_or(Value::Null, Value::Object))
            })),
            on_added_temp_data: Some(Arc::new(move |any: &Value, cx: &mut App| {
                let Some(row) = AnyDictionaryBridge::dictionary(Some(any)) else {
                    return;
                };
                let temp_id = row.get(chat_key::ID).and_then(Value::as_str).map(str::to_owned);
                *added_temp_id.lock() = temp_id.clone();
                let dto = SaiJSONCodec::decode::<RiseStoredChat>(Some(any));
                ChatServicesStore::shared(cx).update(cx, |services, cx| {
                    services.pending_create_chat_temp_id = temp_id;
                    cx.notify();
                    if let Some(dto) = dto {
                        services.insert_chat_at_top_if_needed(dto.to_chat_info(), cx);
                    }
                });
            })),
            ..SaiWsOptimisticUpdate::default()
        };

        let reflect_pending_in_ui = local_logo_file.is_none();
        if let Some(local_logo_file) = local_logo_file {
            let progress_sheet = sheet.clone();
            let error_sheet = sheet.clone();
            let file_error = on_error.clone();
            let slice = self.create_chat_sai.clone();
            optimistic.files = Some(ChatServicesStore::make_logo_optimistic_files(
                local_logo_file,
                Rc::new(move |progress: f64, cx: &mut App| {
                    progress_sheet.update(cx, |sheet, cx| sheet.patch_logo_upload(progress, true, cx));
                }),
                Rc::new(move |error: &anyhow::Error, cx: &mut App| {
                    error_sheet.update(cx, |sheet, cx| {
                        sheet.patch_logo_upload(0.0, false, cx);
                        sheet.is_creating = false;
                        cx.notify();
                    });
                    if let Some(on_error) = &file_error {
                        on_error(CanonErrors::refusal_text(error), cx);
                    }
                }),
                Rc::new(move |error: &anyhow::Error, _: &mut App| slice.mark_rejected(error)),
            ));
            sheet.update(cx, |sheet, cx| sheet.patch_logo_upload(0.0, true, cx));
        }

        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::chat::SERVICE.to_owned(),
            method: rise_wire_methods::chat::CREATE_CHAT.to_owned(),
            cache_key: Some(cache_key),
            reload_if_cached: true,
            allow_concurrent_while_pending: false,
            reflect_pending_in_ui,
            send_immediately: true,
            store_response_in_cache: true,
            persist_cache_to_disk: false,
            path_to_array: Some(chat_key::CHATS.to_owned()),
            path_to_id: Some(chat_key::ID.to_owned()),
            optimistic_update: Some(optimistic),
            ..SaiWsRequestOptionsInit::default()
        });

        let success_sheet = sheet.clone();
        let success_temp_id = coordinator_temp_id.clone();
        let on_wire_success: SaiWsOnSuccess = Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
            let result = typed_payload::<CreateChatResultDTO>(payload);
            let full_chat = result.as_ref().and_then(|result| result.full_chat());
            let real_id = full_chat
                .as_ref()
                .map(|chat| chat.id.clone())
                .or_else(|| result.as_ref().map(|result| result.chat_id.clone()))
                .unwrap_or_default();
            let temp_id = success_temp_id.lock().clone();
            ChatServicesStore::shared(cx).update(cx, |services, cx| {
                if let Some(temp_id) = temp_id.filter(|temp_id| GetSaiServerId::is_temp_id(Some(temp_id)))
                    && !real_id.is_empty()
                {
                    services.finalize_created_chat(&temp_id, &real_id, cx);
                }
                if let Some(full_chat) = &full_chat {
                    services.upsert_chat(full_chat.to_chat_info(), cx);
                }
            });
            success_sheet.update(cx, |sheet, cx| {
                sheet.patch_logo_upload(100.0, false, cx);
                sheet.is_creating = false;
                cx.notify();
            });
            if let Some(on_success) = &on_success {
                on_success(real_id, cx);
            }
        });
        let on_wire_error: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            let temp_id = coordinator_temp_id.lock().clone();
            ChatServicesStore::shared(cx).update(cx, |services, cx| {
                if temp_id.is_some() && services.pending_create_chat_temp_id == temp_id {
                    services.pending_create_chat_temp_id = None;
                    cx.notify();
                }
            });
            sheet.update(cx, |sheet, cx| {
                sheet.patch_logo_upload(0.0, false, cx);
                sheet.is_creating = false;
                cx.notify();
            });
            log_chat_failure(format!("create_chat error: {error}"), cx);
            if let Some(on_error) = &on_error {
                on_error(CanonErrors::refusal_text(error), cx);
            }
        });

        SaiWs::request_slice(&self.create_chat_sai, request, options, Some(on_wire_success), Some(on_wire_error), cx);
    }

    fn abandon_create_chat(sheet: &Entity<CreateChatSheetState>, cx: &mut Context<Self>) {
        sheet.update(cx, |sheet, cx| {
            sheet.is_creating = false;
            cx.notify();
        });
    }

    pub fn open_direct_chat_action(
        &mut self,
        participant: User,
        on_open: Box<dyn FnOnce(String, &mut App)>,
        on_error: Option<Box<dyn FnOnce(String, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        let Some(user_id) = effective_user_id_for_wire(cx) else {
            return;
        };

        let target_id = trimming_whitespaces_and_newlines(&participant.id).to_owned();

        if target_id.is_empty() || target_id == user_id {
            return;
        }

        let services = ChatServicesStore::shared(cx);
        let existing_id = services.read(cx).known_direct_chat_id(&target_id, cx);
        if !existing_id.is_empty() {
            cx.defer(move |cx| on_open(existing_id, cx));
            return;
        }

        let begins_opening = services.update(cx, |services, cx| {
            services.enqueue_direct_chat_waiter(&target_id, on_open, on_error);
            services.begin_opening_direct_chat(&target_id, cx)
        });
        if !begins_opening {
            return;
        }

        let mut body = rise_request::chat::CreateChat::new(participant.name.clone(), "PRIVATE".to_owned());
        body.banner_url = Some(String::new());
        body.description = Some(String::new());
        body.forward_in_chat_enabled = Some(true);
        body.is_public = Some(false);
        body.logo_url = Some(String::new());
        body.participants = Some(vec![user_id, target_id.clone()]);

        let success_target = target_id.clone();
        let on_success: SaiWsOnSuccess = Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
            let result = typed_payload::<CreateChatResultDTO>(payload);
            let full_chat = result.as_ref().and_then(|result| result.full_chat());
            let real_id = full_chat
                .as_ref()
                .map(|chat| chat.id.clone())
                .or_else(|| result.as_ref().map(|result| result.chat_id.clone()))
                .unwrap_or_default();
            let services = ChatServicesStore::shared(cx);
            if real_id.is_empty() {
                let text = tr("retry_later_error");
                services.update(cx, |services, cx| {
                    services.finish_opening_direct_chat(&success_target, None, Some(&text), cx);
                });
                return;
            }

            if let Some(full_chat) = &full_chat {
                services.update(cx, |services, cx| services.adopt_reopened_chat(full_chat.to_chat_info(), cx));
            } else {
                services.update(cx, |services, cx| services.clear_pending_delete_marks(&real_id, cx));
                ChatActionsStore::shared(cx).update(cx, |actions, cx| actions.get_chat_by_id_action(&real_id, false, cx));
            }
            services.update(cx, |services, cx| {
                services.finish_opening_direct_chat(&success_target, Some(&real_id), None, cx);
            });
        });
        let on_failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            log_chat_failure(format!("open_direct_chat({target_id}) error: {error}"), cx);
            if Self::is_private_chat_refusal(error)
                && let Some(hooks) = chat_stores_user_hooks()
            {
                (hooks.note_peer_refuses_viewer_messages)(&target_id, cx);
            }
            let text = CanonErrors::refusal_text(error);
            ChatServicesStore::shared(cx).update(cx, |services, cx| {
                services.finish_opening_direct_chat(&target_id, None, Some(&text), cx);
            });
        });

        SaiWs::request_slice(
            &self.open_direct_chat_sai,
            body,
            immediate_options(rise_wire_methods::chat::CREATE_CHAT),
            Some(on_success),
            Some(on_failure),
            cx,
        );
    }

    fn is_private_chat_refusal(error: &anyhow::Error) -> bool {
        CanonErrors::failure(error).and_then(|failure| failure.code).as_deref()
            == Some(ErrorCode::ChatPermissionDenied.wire())
    }
}
