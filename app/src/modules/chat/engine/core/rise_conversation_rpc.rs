use std::borrow::Borrow;
use std::collections::BTreeMap;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use rise_canon::{RiseWireRequestMetadata, rise_key, rise_wire_methods};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_endpoints::{EndpointKind as RiseEndpointKind, TIMEOUT_DEFAULT_MS};
use rise_engine::rise_core::rise_identifiers::RiseRequestID;
use rise_engine::{
    RiseEngine, RiseJSONValue, RiseWireError, RiseWireMethod, RiseWireMethodDescriptor, RiseWireReplayPolicy,
};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::core::rise_engine_bridge::rise_canon_endpoints::{
    RiseCanonEndpoints, RiseEndpoint, RiseEndpointClientTimeout,
};
use crate::modules::auth::services::app_attest_client::AppAttestClient;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum RiseConversationEndpointKind {
    Read,
    Mutation,
    RequestIDMutation,
    IdempotentMutation { field: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RiseConversationEndpoint {
    pub service: String,
    pub method: String,
    pub kind: RiseConversationEndpointKind,
    pub timeout: Duration,
    pub survives_suspension: bool,
}

impl RiseConversationEndpoint {
    /// new endpoint with defaults, with_timeout and with_survives_suspension set the rest
    pub fn new(
        service: impl Into<String>,
        method: impl Into<String>,
        kind: RiseConversationEndpointKind,
    ) -> Self {
        Self {
            service: service.into(),
            method: method.into(),
            kind,
            timeout: RiseConversationEndpoints::default_timeout(),
            survives_suspension: false,
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn with_survives_suspension(mut self, survives_suspension: bool) -> Self {
        self.survives_suspension = survives_suspension;
        self
    }

    /// descriptor from the canon catalogue, kind and deadline come from there
    /// expected only checks the declaration
    /// timeout_override is for a method declared twice with different values
    #[track_caller]
    pub fn canonical(
        service: &str,
        method: &str,
        expected: RiseEndpointKind,
        timeout_override: Option<Duration>,
        survives_suspension: bool,
    ) -> Self {
        let entry = RiseCanonEndpoints::required(service, method);
        debug_assert!(
            entry.kind == expected,
            "{service}.{method}: canon declares {:?}, not {expected:?}",
            entry.kind
        );
        Self {
            service: entry.service.to_owned(),
            method: entry.method.to_owned(),
            kind: RiseConversationEndpointKind::from(entry),
            timeout: timeout_override.unwrap_or_else(|| entry.client_timeout()),
            survives_suspension,
        }
    }

    pub fn descriptor(&self) -> RiseWireMethodDescriptor {
        let replay_policy = match &self.kind {
            RiseConversationEndpointKind::Read => RiseWireReplayPolicy::ReadOnly,
            RiseConversationEndpointKind::Mutation => RiseWireReplayPolicy::Never,
            RiseConversationEndpointKind::RequestIDMutation => RiseWireReplayPolicy::RequestID,
            RiseConversationEndpointKind::IdempotentMutation { field } => {
                RiseWireReplayPolicy::IdempotencyKey { field: field.clone() }
            }
        };
        RiseWireMethodDescriptor::new(self.service.clone(), self.method.clone())
            .with_timeout(self.timeout)
            .with_replay_policy(replay_policy)
            .with_survives_suspension(self.survives_suspension)
    }
}

/// full conversation rpc catalogue
/// keeping it in one place stops a migrated request from falling back to the old transport
pub enum RiseConversationEndpoints {}

impl RiseConversationEndpoints {
    pub fn get_link_preview() -> RiseConversationEndpoint {
        Self::read(
            rise_wire_methods::link_preview::SERVICE,
            rise_wire_methods::link_preview::GET_LINK_PREVIEW,
            None,
        )
    }


    pub fn get_chats() -> RiseConversationEndpoint {
        Self::read(rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::GET_CHATS, None)
    }

    pub fn get_common_chats() -> RiseConversationEndpoint {
        Self::read(rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::GET_COMMON_CHATS, None)
    }

    pub fn get_chat_by_id() -> RiseConversationEndpoint {
        Self::read(rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::GET_CHAT_BY_ID, None)
    }

    pub fn get_archived_chats() -> RiseConversationEndpoint {
        Self::read(rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::GET_ARCHIVED_CHATS, None)
    }

    pub fn get_unread_summary() -> RiseConversationEndpoint {
        Self::read(rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::GET_UNREAD_SUMMARY, None)
    }

    pub fn get_message_requests() -> RiseConversationEndpoint {
        Self::read(rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::GET_MESSAGE_REQUESTS, None)
    }

    pub fn get_chat_folders() -> RiseConversationEndpoint {
        Self::read(rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::GET_CHAT_FOLDERS, None)
    }

    pub fn get_chat_members() -> RiseConversationEndpoint {
        Self::read(rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::GET_CHAT_MEMBERS, None)
    }

    pub fn get_admin_members() -> RiseConversationEndpoint {
        Self::read(rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::GET_ADMIN_MEMBERS, None)
    }

    pub fn get_blocked_members() -> RiseConversationEndpoint {
        Self::read(rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::GET_BLOCKED_MEMBERS, None)
    }

    pub fn get_chat_actions_history() -> RiseConversationEndpoint {
        Self::read(rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::GET_CHAT_ACTIONS_HISTORY, None)
    }

    pub fn get_topics() -> RiseConversationEndpoint {
        Self::read(rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::GET_TOPICS, None)
    }

    pub fn get_invite_links() -> RiseConversationEndpoint {
        Self::read(rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::GET_INVITE_LINKS, None)
    }

    pub fn get_join_requests() -> RiseConversationEndpoint {
        Self::read(rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::GET_JOIN_REQUESTS, None)
    }

    pub fn get_chat_preview_by_invite_link() -> RiseConversationEndpoint {
        Self::read(
            rise_wire_methods::chat::SERVICE,
            rise_wire_methods::chat::GET_CHAT_PREVIEW_BY_INVITE_LINK,
            None,
        )
    }

    pub fn get_chat_settings() -> RiseConversationEndpoint {
        Self::read(rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::GET_CHAT_SETTINGS, None)
    }

    pub fn get_linked_chat() -> RiseConversationEndpoint {
        Self::read(
            rise_wire_methods::chat::SERVICE,
            rise_wire_methods::chat::GET_LINKED_CHAT_BY_CHAT_ID,
            None,
        )
    }

    pub fn get_chat_analytics() -> RiseConversationEndpoint {
        Self::read(rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::GET_CHAT_ANALYTICS, None)
    }

    pub fn get_forum_structure() -> RiseConversationEndpoint {
        Self::read(rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::GET_FORUM_STRUCTURE, None)
    }

    pub fn get_effective_permissions() -> RiseConversationEndpoint {
        Self::read(rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::GET_EFFECTIVE_PERMISSIONS, None)
    }

    pub fn get_topic_overwrites() -> RiseConversationEndpoint {
        Self::read(rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::GET_TOPIC_OVERWRITES, None)
    }

    pub fn get_section_overwrites() -> RiseConversationEndpoint {
        Self::read(rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::GET_SECTION_OVERWRITES, None)
    }

    pub fn get_share_targets() -> RiseConversationEndpoint {
        Self::read(rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::GET_SHARE_TARGETS, None)
    }

    pub fn get_share_topics() -> RiseConversationEndpoint {
        Self::read(rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::GET_SHARE_TOPICS, None)
    }

    pub fn get_group_voice_channel() -> RiseConversationEndpoint {
        Self::read(rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::GET_GROUP_VOICE_CHANNEL, None)
    }

    pub fn check_tag_exist() -> RiseConversationEndpoint {
        Self::read(rise_wire_methods::tag::SERVICE, rise_wire_methods::tag::CHECK_TAG_EXIST, None)
    }

    pub fn global_search_by_tag() -> RiseConversationEndpoint {
        Self::read(rise_wire_methods::tag::SERVICE, rise_wire_methods::tag::GLOBAL_SEARCH_BY_TAG, None)
    }


    pub fn create_chat() -> RiseConversationEndpoint {
        Self::mutation(rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::CREATE_CHAT, false)
    }

    pub fn update_chat() -> RiseConversationEndpoint {
        Self::mutation(rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::UPDATE_CHAT, false)
    }

    pub fn set_chat_wallpaper() -> RiseConversationEndpoint {
        Self::mutation(rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::SET_CHAT_WALLPAPER, false)
    }

    pub fn add_chat_wallpaper() -> RiseConversationEndpoint {
        Self::mutation(rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::ADD_CHAT_WALLPAPER, false)
    }

    pub fn get_chat_wallpapers() -> RiseConversationEndpoint {
        Self::read(rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::GET_CHAT_WALLPAPERS, None)
    }

    pub fn delete_chat_wallpaper() -> RiseConversationEndpoint {
        Self::mutation(
            rise_wire_methods::chat::SERVICE,
            rise_wire_methods::chat::DELETE_CHAT_WALLPAPER,
            false,
        )
    }

    pub fn mute_chat() -> RiseConversationEndpoint {
        Self::mutation(rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::MUTE_CHAT, false)
    }

    pub fn unmute_chat() -> RiseConversationEndpoint {
        Self::mutation(rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::UNMUTE_CHAT, false)
    }

    pub fn update_chat_notifications() -> RiseConversationEndpoint {
        Self::mutation(
            rise_wire_methods::chat::SERVICE,
            rise_wire_methods::chat::UPDATE_CHAT_NOTIFICATIONS,
            false,
        )
    }

    pub fn leave_chat() -> RiseConversationEndpoint {
        Self::mutation(rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::LEAVE_CHAT, false)
    }

    pub fn delete_chat() -> RiseConversationEndpoint {
        Self::mutation(rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::DELETE_CHAT, true)
    }

    pub fn unpin_chat() -> RiseConversationEndpoint {
        Self::mutation(rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::UNPIN_CHAT, false)
    }

    pub fn reorder_pinned_chats() -> RiseConversationEndpoint {
        Self::mutation(rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::REORDER_PINNED_CHATS, false)
    }

    pub fn archive_chat() -> RiseConversationEndpoint {
        Self::request_id_mutation(rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::ARCHIVE_CHAT)
    }

    pub fn unarchive_chat() -> RiseConversationEndpoint {
        Self::request_id_mutation(rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::UNARCHIVE_CHAT)
    }

    pub fn resolve_message_request() -> RiseConversationEndpoint {
        Self::mutation(
            rise_wire_methods::chat::SERVICE,
            rise_wire_methods::chat::RESOLVE_MESSAGE_REQUEST,
            false,
        )
    }

    pub fn delete_all_message_requests() -> RiseConversationEndpoint {
        Self::mutation(
            rise_wire_methods::chat::SERVICE,
            rise_wire_methods::chat::DELETE_ALL_MESSAGE_REQUESTS,
            false,
        )
    }

    pub fn create_chat_folder() -> RiseConversationEndpoint {
        Self::mutation(rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::CREATE_CHAT_FOLDER, false)
    }

    pub fn edit_chat_folder() -> RiseConversationEndpoint {
        Self::mutation(rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::EDIT_CHAT_FOLDER, false)
    }

    pub fn delete_chat_folder() -> RiseConversationEndpoint {
        Self::mutation(rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::DELETE_CHAT_FOLDER, false)
    }

    pub fn reorder_chat_folders() -> RiseConversationEndpoint {
        Self::mutation(rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::REORDER_CHAT_FOLDERS, false)
    }

    pub fn ban_member() -> RiseConversationEndpoint {
        Self::mutation(rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::BAN_MEMBER, false)
    }

    pub fn kick_member() -> RiseConversationEndpoint {
        Self::mutation(rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::KICK_MEMBER, false)
    }

    pub fn add_member() -> RiseConversationEndpoint {
        Self::mutation(rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::ADD_MEMBER, false)
    }

    pub fn edit_member_rules() -> RiseConversationEndpoint {
        Self::mutation(rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::EDIT_MEMBER_RULES, false)
    }

    pub fn edit_member() -> RiseConversationEndpoint {
        Self::mutation(rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::EDIT_MEMBER, false)
    }

    pub fn transfer_ownership() -> RiseConversationEndpoint {
        Self::mutation(rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::TRANSFER_OWNERSHIP, false)
    }

    pub fn unban_member() -> RiseConversationEndpoint {
        Self::mutation(rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::UNBAN_MEMBER, false)
    }

    pub fn create_topic() -> RiseConversationEndpoint {
        Self::mutation(rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::CREATE_TOPIC, false)
    }

    pub fn delete_topic() -> RiseConversationEndpoint {
        Self::mutation(rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::DELETE_TOPIC, false)
    }

    pub fn update_topic() -> RiseConversationEndpoint {
        Self::mutation(rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::UPDATE_TOPIC, false)
    }

    pub fn update_topic_notifications() -> RiseConversationEndpoint {
        Self::mutation(
            rise_wire_methods::chat::SERVICE,
            rise_wire_methods::chat::UPDATE_TOPIC_NOTIFICATIONS,
            false,
        )
    }

    pub fn create_invite_link() -> RiseConversationEndpoint {
        Self::mutation(rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::CREATE_INVITE_LINK, false)
    }

    pub fn get_voice_invite_link() -> RiseConversationEndpoint {
        Self::mutation(rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::GET_VOICE_INVITE_LINK, false)
    }

    pub fn revoke_invite_link() -> RiseConversationEndpoint {
        Self::mutation(rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::REVOKE_INVITE_LINK, false)
    }

    pub fn join_chat_by_link() -> RiseConversationEndpoint {
        Self::mutation(rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::JOIN_CHAT_BY_LINK, false)
    }

    pub fn join_chat() -> RiseConversationEndpoint {
        Self::mutation(rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::JOIN_CHAT, false)
    }

    pub fn request_join_chat() -> RiseConversationEndpoint {
        Self::mutation(rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::REQUEST_JOIN_CHAT, false)
    }

    pub fn approve_join_request() -> RiseConversationEndpoint {
        Self::mutation(rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::APPROVE_JOIN_REQUEST, false)
    }

    pub fn reject_join_request() -> RiseConversationEndpoint {
        Self::mutation(rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::REJECT_JOIN_REQUEST, false)
    }

    pub fn update_chat_settings() -> RiseConversationEndpoint {
        Self::mutation(rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::UPDATE_CHAT_SETTINGS, false)
    }

    pub fn link_chat() -> RiseConversationEndpoint {
        Self::mutation(rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::LINK_CHAT, false)
    }

    pub fn unlink_chat() -> RiseConversationEndpoint {
        Self::mutation(rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::UNLINK_CHAT, false)
    }

    pub fn create_role() -> RiseConversationEndpoint {
        Self::mutation(rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::CREATE_ROLE, false)
    }

    pub fn update_role() -> RiseConversationEndpoint {
        Self::mutation(rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::UPDATE_ROLE, false)
    }

    pub fn delete_role() -> RiseConversationEndpoint {
        Self::mutation(rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::DELETE_ROLE, false)
    }

    pub fn reorder_roles() -> RiseConversationEndpoint {
        Self::mutation(rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::REORDER_ROLES, false)
    }

    pub fn assign_role() -> RiseConversationEndpoint {
        Self::mutation(rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::ASSIGN_ROLE, false)
    }

    pub fn unassign_role() -> RiseConversationEndpoint {
        Self::mutation(rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::UNASSIGN_ROLE, false)
    }

    pub fn create_section() -> RiseConversationEndpoint {
        Self::mutation(rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::CREATE_SECTION, false)
    }

    pub fn update_section() -> RiseConversationEndpoint {
        Self::mutation(rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::UPDATE_SECTION, false)
    }

    pub fn delete_section() -> RiseConversationEndpoint {
        Self::mutation(rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::DELETE_SECTION, false)
    }

    pub fn reorder_sections() -> RiseConversationEndpoint {
        Self::mutation(rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::REORDER_SECTIONS, false)
    }

    pub fn set_topic_overwrite() -> RiseConversationEndpoint {
        Self::mutation(rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::SET_TOPIC_OVERWRITE, false)
    }

    pub fn set_section_overwrite() -> RiseConversationEndpoint {
        Self::mutation(
            rise_wire_methods::chat::SERVICE,
            rise_wire_methods::chat::SET_SECTION_OVERWRITE,
            false,
        )
    }

    pub fn remove_topic_overwrite() -> RiseConversationEndpoint {
        Self::mutation(
            rise_wire_methods::chat::SERVICE,
            rise_wire_methods::chat::REMOVE_TOPIC_OVERWRITE,
            false,
        )
    }

    pub fn remove_section_overwrite() -> RiseConversationEndpoint {
        Self::mutation(
            rise_wire_methods::chat::SERVICE,
            rise_wire_methods::chat::REMOVE_SECTION_OVERWRITE,
            false,
        )
    }

    pub fn set_member_permissions() -> RiseConversationEndpoint {
        Self::mutation(
            rise_wire_methods::chat::SERVICE,
            rise_wire_methods::chat::SET_MEMBER_PERMISSIONS,
            false,
        )
    }

    pub fn update_group_voice_channel() -> RiseConversationEndpoint {
        Self::mutation(
            rise_wire_methods::chat::SERVICE,
            rise_wire_methods::chat::UPDATE_GROUP_VOICE_CHANNEL,
            false,
        )
    }


    pub fn get_messages() -> RiseConversationEndpoint {
        Self::read(rise_wire_methods::message::SERVICE, rise_wire_methods::message::GET_MESSAGES, None)
    }

    pub fn get_messages_around() -> RiseConversationEndpoint {
        Self::read(rise_wire_methods::message::SERVICE, rise_wire_methods::message::GET_MESSAGES_AROUND, None)
    }

    pub fn get_message_by_id() -> RiseConversationEndpoint {
        Self::read(rise_wire_methods::message::SERVICE, rise_wire_methods::message::GET_MESSAGE_BY_ID, None)
    }

    pub fn get_chat_calendar() -> RiseConversationEndpoint {
        Self::read(rise_wire_methods::message::SERVICE, rise_wire_methods::message::GET_CHAT_CALENDAR, None)
    }

    pub fn get_pinned_messages() -> RiseConversationEndpoint {
        Self::read(rise_wire_methods::message::SERVICE, rise_wire_methods::message::GET_PINNED_MESSAGES, None)
    }

    pub fn get_media_messages() -> RiseConversationEndpoint {
        Self::read(rise_wire_methods::message::SERVICE, rise_wire_methods::message::GET_MEDIA_MESSAGES, None)
    }

    pub fn get_file_messages() -> RiseConversationEndpoint {
        Self::read(rise_wire_methods::message::SERVICE, rise_wire_methods::message::GET_FILE_MESSAGES, None)
    }

    pub fn get_music_messages() -> RiseConversationEndpoint {
        Self::read(rise_wire_methods::message::SERVICE, rise_wire_methods::message::GET_MUSIC_MESSAGES, None)
    }

    pub fn get_audio_messages() -> RiseConversationEndpoint {
        Self::read(rise_wire_methods::message::SERVICE, rise_wire_methods::message::GET_AUDIO_MESSAGES, None)
    }

    pub fn get_gif_messages() -> RiseConversationEndpoint {
        Self::read(rise_wire_methods::message::SERVICE, rise_wire_methods::message::GET_GIF_MESSAGES, None)
    }

    pub fn get_favorite_messages() -> RiseConversationEndpoint {
        Self::read(rise_wire_methods::message::SERVICE, rise_wire_methods::message::GET_FAV_MESSAGES, None)
    }

    pub fn get_link_messages() -> RiseConversationEndpoint {
        Self::read(rise_wire_methods::message::SERVICE, rise_wire_methods::message::GET_LINK_MESSAGES, None)
    }

    pub fn get_message_read_users() -> RiseConversationEndpoint {
        Self::read(
            rise_wire_methods::message::SERVICE,
            rise_wire_methods::message::GET_MESSAGE_READ_USERS,
            None,
        )
    }

    pub fn get_poll_voters() -> RiseConversationEndpoint {
        Self::read(rise_wire_methods::message::SERVICE, rise_wire_methods::message::GET_POLL_VOTERS, None)
    }

    pub fn get_unread_marks() -> RiseConversationEndpoint {
        Self::read(rise_wire_methods::message::SERVICE, rise_wire_methods::message::GET_UNREAD_MARKS, None)
    }

    pub fn search_messages() -> RiseConversationEndpoint {
        Self::read(
            rise_wire_methods::message::SERVICE,
            rise_wire_methods::message::SEARCH_MESSAGES_GLOBAL,
            None,
        )
    }

    pub fn search_chat_messages() -> RiseConversationEndpoint {
        Self::read(rise_wire_methods::message::SERVICE, rise_wire_methods::message::SEARCH_MESSAGES, None)
    }

    pub fn global_search() -> RiseConversationEndpoint {
        Self::read(
            rise_wire_methods::search::SERVICE,
            rise_wire_methods::search::GLOBAL_SEARCH,
            Some(Self::default_timeout()),
        )
    }

    pub fn search_posts() -> RiseConversationEndpoint {
        Self::read(
            rise_wire_methods::post::SERVICE,
            rise_wire_methods::post::SEARCH_POSTS,
            Some(Self::default_timeout()),
        )
    }


    pub fn create_message() -> RiseConversationEndpoint {
        Self::mutation(rise_wire_methods::message::SERVICE, rise_wire_methods::message::CREATE_MESSAGE, false)
    }

    pub fn edit_message() -> RiseConversationEndpoint {
        Self::mutation(rise_wire_methods::message::SERVICE, rise_wire_methods::message::EDIT_MESSAGE, false)
    }

    pub fn delete_messages() -> RiseConversationEndpoint {
        Self::mutation(
            rise_wire_methods::message::SERVICE,
            rise_wire_methods::message::DELETE_MESSAGES,
            false,
        )
    }

    pub fn forward_messages() -> RiseConversationEndpoint {
        Self::idempotent(rise_wire_methods::message::SERVICE, rise_wire_methods::message::FORWARD_MESSAGES)
    }

    pub fn share_payload() -> RiseConversationEndpoint {
        Self::idempotent(rise_wire_methods::message::SERVICE, rise_wire_methods::message::SHARE_PAYLOAD)
    }

    pub fn delete_all_messages_from_user() -> RiseConversationEndpoint {
        Self::mutation(
            rise_wire_methods::message::SERVICE,
            rise_wire_methods::message::DELETE_ALL_MESSAGES_FROM_USER,
            false,
        )
    }

    pub fn pin_message() -> RiseConversationEndpoint {
        Self::mutation(rise_wire_methods::message::SERVICE, rise_wire_methods::message::PIN_MESSAGE, false)
    }

    pub fn unpin_message() -> RiseConversationEndpoint {
        Self::mutation(rise_wire_methods::message::SERVICE, rise_wire_methods::message::UNPIN_MESSAGE, false)
    }

    pub fn unpin_all_messages() -> RiseConversationEndpoint {
        Self::mutation(
            rise_wire_methods::message::SERVICE,
            rise_wire_methods::message::UNPIN_ALL_MESSAGES,
            false,
        )
    }

    pub fn mark_chat_as_read() -> RiseConversationEndpoint {
        Self::mutation(
            rise_wire_methods::message::SERVICE,
            rise_wire_methods::message::MARK_CHAT_AS_READ,
            false,
        )
    }

    pub fn mark_message_as_mentioned_read() -> RiseConversationEndpoint {
        Self::mutation(
            rise_wire_methods::message::SERVICE,
            rise_wire_methods::message::MARK_MESSAGE_AS_MENTIONED_READ,
            false,
        )
    }

    pub fn mark_reactions_as_read() -> RiseConversationEndpoint {
        Self::mutation(
            rise_wire_methods::message::SERVICE,
            rise_wire_methods::message::MARK_REACTIONS_AS_READ,
            false,
        )
    }

    pub fn clear_unread_marks() -> RiseConversationEndpoint {
        Self::mutation(
            rise_wire_methods::message::SERVICE,
            rise_wire_methods::message::CLEAR_UNREAD_MARKS,
            false,
        )
    }

    pub fn set_chat_unread_mark() -> RiseConversationEndpoint {
        Self::mutation(
            rise_wire_methods::message::SERVICE,
            rise_wire_methods::message::SET_CHAT_UNREAD_MARK,
            false,
        )
    }

    pub fn mark_media_played() -> RiseConversationEndpoint {
        Self::mutation(
            rise_wire_methods::message::SERVICE,
            rise_wire_methods::message::MARK_MEDIA_AS_PLAYED,
            false,
        )
    }

    pub fn vote_poll() -> RiseConversationEndpoint {
        Self::mutation(rise_wire_methods::message::SERVICE, rise_wire_methods::message::VOTE_POLL, false)
    }

    pub fn mark_message_viewed() -> RiseConversationEndpoint {
        Self::mutation(
            rise_wire_methods::message::SERVICE,
            rise_wire_methods::message::MARK_MESSAGE_AS_VIEWED,
            false,
        )
    }

    pub fn typing() -> RiseConversationEndpoint {
        Self::mutation(rise_wire_methods::message::SERVICE, rise_wire_methods::message::MESSAGE_TYPING, false)
    }

    pub fn add_reaction() -> RiseConversationEndpoint {
        Self::mutation(rise_wire_methods::message::SERVICE, rise_wire_methods::message::ADD_REACTION, false)
    }

    pub fn remove_reaction() -> RiseConversationEndpoint {
        Self::mutation(
            rise_wire_methods::message::SERVICE,
            rise_wire_methods::message::REMOVE_REACTION,
            false,
        )
    }

    pub fn send_message_sparks() -> RiseConversationEndpoint {
        Self::idempotent(rise_wire_methods::message::SERVICE, rise_wire_methods::message::SEND_MESSAGE_SPARKS)
    }

    pub fn get_message_sparks() -> RiseConversationEndpoint {
        Self::read(rise_wire_methods::message::SERVICE, rise_wire_methods::message::GET_MESSAGE_SPARKS, None)
    }


    pub fn send_inline_result() -> RiseConversationEndpoint {
        Self::idempotent(rise_wire_methods::bot::SERVICE, rise_wire_methods::bot::SEND_INLINE_RESULT)
    }

    pub fn get_bot_by_tag() -> RiseConversationEndpoint {
        Self::read(rise_wire_methods::bot::SERVICE, rise_wire_methods::bot::GET_BOT_BY_TAG, None)
    }


    pub fn get_default_sticker_packs() -> RiseConversationEndpoint {
        Self::read(rise_wire_methods::stickers::SERVICE, rise_wire_methods::stickers::GET_DEFAULT_PACKS, None)
    }

    pub fn get_saved_sticker_packs() -> RiseConversationEndpoint {
        Self::read(rise_wire_methods::stickers::SERVICE, rise_wire_methods::stickers::GET_SAVED_PACKS, None)
    }

    pub fn get_my_sticker_packs() -> RiseConversationEndpoint {
        Self::read(rise_wire_methods::stickers::SERVICE, rise_wire_methods::stickers::GET_MY_PACKS, None)
    }

    pub fn get_stickers_by_pack_id() -> RiseConversationEndpoint {
        Self::read(
            rise_wire_methods::stickers::SERVICE,
            rise_wire_methods::stickers::GET_STICKERS_BY_PACK_ID,
            None,
        )
    }

    pub fn get_stickers_by_pack_link() -> RiseConversationEndpoint {
        Self::read(
            rise_wire_methods::stickers::SERVICE,
            rise_wire_methods::stickers::GET_STICKERS_BY_PACK_LINK,
            None,
        )
    }

    pub fn get_custom_emoji_by_ids() -> RiseConversationEndpoint {
        Self::read(
            rise_wire_methods::stickers::SERVICE,
            rise_wire_methods::stickers::GET_CUSTOM_EMOJI_BY_IDS,
            None,
        )
    }

    pub fn get_sticker_favourites() -> RiseConversationEndpoint {
        Self::read(
            rise_wire_methods::stickers::SERVICE,
            rise_wire_methods::stickers::GET_USER_FAVOURITES,
            None,
        )
    }

    pub fn get_sticker_recents() -> RiseConversationEndpoint {
        Self::read(rise_wire_methods::stickers::SERVICE, rise_wire_methods::stickers::GET_USER_RECENT, None)
    }

    pub fn get_gif_favourites() -> RiseConversationEndpoint {
        Self::read(
            rise_wire_methods::stickers::SERVICE,
            rise_wire_methods::stickers::GET_GIF_FAVOURITES,
            None,
        )
    }

    pub fn search_gifs() -> RiseConversationEndpoint {
        Self::read(rise_wire_methods::stickers::SERVICE, rise_wire_methods::stickers::SEARCH_GIFS, None)
    }

    pub fn add_sticker_favourite() -> RiseConversationEndpoint {
        Self::mutation(
            rise_wire_methods::stickers::SERVICE,
            rise_wire_methods::stickers::ADD_STICKER_FAVOURITE,
            false,
        )
    }

    pub fn remove_sticker_favourite() -> RiseConversationEndpoint {
        Self::mutation(
            rise_wire_methods::stickers::SERVICE,
            rise_wire_methods::stickers::REMOVE_STICKER_FAVOURITE,
            false,
        )
    }

    pub fn write_sticker_recent() -> RiseConversationEndpoint {
        Self::mutation(rise_wire_methods::stickers::SERVICE, rise_wire_methods::stickers::WRITE_RECENT, false)
    }

    pub fn clear_sticker_recent() -> RiseConversationEndpoint {
        Self::mutation(rise_wire_methods::stickers::SERVICE, rise_wire_methods::stickers::CLEAR_RECENT, false)
    }

    pub fn add_gif_favourite() -> RiseConversationEndpoint {
        Self::mutation(
            rise_wire_methods::stickers::SERVICE,
            rise_wire_methods::stickers::ADD_GIF_FAVOURITE,
            false,
        )
    }

    pub fn remove_gif_favourite() -> RiseConversationEndpoint {
        Self::mutation(
            rise_wire_methods::stickers::SERVICE,
            rise_wire_methods::stickers::REMOVE_GIF_FAVOURITE,
            false,
        )
    }

    pub fn save_sticker_pack() -> RiseConversationEndpoint {
        Self::mutation(rise_wire_methods::stickers::SERVICE, rise_wire_methods::stickers::SAVE_PACK, false)
    }

    pub fn remove_saved_sticker_pack() -> RiseConversationEndpoint {
        Self::mutation(
            rise_wire_methods::stickers::SERVICE,
            rise_wire_methods::stickers::REMOVE_SAVED_PACK,
            false,
        )
    }

    pub fn delete_sticker_pack() -> RiseConversationEndpoint {
        Self::mutation(rise_wire_methods::stickers::SERVICE, rise_wire_methods::stickers::DELETE_PACK, false)
    }

    pub fn edit_sticker_pack() -> RiseConversationEndpoint {
        Self::mutation(rise_wire_methods::stickers::SERVICE, rise_wire_methods::stickers::EDIT_PACK, false)
    }

    pub fn create_sticker_pack() -> RiseConversationEndpoint {
        Self::idempotent(rise_wire_methods::stickers::SERVICE, rise_wire_methods::stickers::CREATE_PACK)
    }

    pub fn create_sticker() -> RiseConversationEndpoint {
        Self::idempotent(rise_wire_methods::stickers::SERVICE, rise_wire_methods::stickers::CREATE_STICKER)
    }

    pub fn edit_sticker() -> RiseConversationEndpoint {
        Self::idempotent(rise_wire_methods::stickers::SERVICE, rise_wire_methods::stickers::EDIT_STICKER)
    }

    pub fn delete_sticker() -> RiseConversationEndpoint {
        Self::mutation(
            rise_wire_methods::stickers::SERVICE,
            rise_wire_methods::stickers::DELETE_STICKER,
            false,
        )
    }

    pub fn reorder_stickers() -> RiseConversationEndpoint {
        Self::mutation(
            rise_wire_methods::stickers::SERVICE,
            rise_wire_methods::stickers::REORDER_STICKERS,
            false,
        )
    }

    pub fn reorder_saved_packs() -> RiseConversationEndpoint {
        Self::mutation(rise_wire_methods::stickers::SERVICE, rise_wire_methods::stickers::REORDER_SAVED_PACKS, false)
    }

    pub fn create_pack_invite_link() -> RiseConversationEndpoint {
        Self::mutation(rise_wire_methods::stickers::SERVICE, rise_wire_methods::stickers::CREATE_PACK_INVITE_LINK, false)
    }

    pub fn edit_pack_invite_link() -> RiseConversationEndpoint {
        Self::mutation(rise_wire_methods::stickers::SERVICE, rise_wire_methods::stickers::EDIT_PACK_INVITE_LINK, false)
    }

    pub fn revoke_pack_invite_link() -> RiseConversationEndpoint {
        Self::mutation(rise_wire_methods::stickers::SERVICE, rise_wire_methods::stickers::REVOKE_PACK_INVITE_LINK, false)
    }

    pub fn get_pack_invite_links() -> RiseConversationEndpoint {
        Self::read(rise_wire_methods::stickers::SERVICE, rise_wire_methods::stickers::GET_PACK_INVITE_LINKS, None)
    }

    pub fn get_pack_invite_link_joiners() -> RiseConversationEndpoint {
        Self::read(rise_wire_methods::stickers::SERVICE, rise_wire_methods::stickers::GET_PACK_INVITE_LINK_JOINERS, None)
    }

    pub fn get_pack_invite_preview() -> RiseConversationEndpoint {
        Self::read(rise_wire_methods::stickers::SERVICE, rise_wire_methods::stickers::GET_PACK_INVITE_PREVIEW, None)
    }

    pub fn join_pack_by_link() -> RiseConversationEndpoint {
        Self::mutation(rise_wire_methods::stickers::SERVICE, rise_wire_methods::stickers::JOIN_PACK_BY_LINK, false)
    }

    pub fn get_pack_join_requests() -> RiseConversationEndpoint {
        Self::read(rise_wire_methods::stickers::SERVICE, rise_wire_methods::stickers::GET_PACK_JOIN_REQUESTS, None)
    }

    pub fn review_pack_join_request() -> RiseConversationEndpoint {
        Self::mutation(rise_wire_methods::stickers::SERVICE, rise_wire_methods::stickers::REVIEW_PACK_JOIN_REQUEST, false)
    }

    pub fn get_pack_coauthors() -> RiseConversationEndpoint {
        Self::read(rise_wire_methods::stickers::SERVICE, rise_wire_methods::stickers::GET_PACK_COAUTHORS, None)
    }

    pub fn update_pack_coauthor_rights() -> RiseConversationEndpoint {
        Self::mutation(rise_wire_methods::stickers::SERVICE, rise_wire_methods::stickers::UPDATE_PACK_COAUTHOR_RIGHTS, false)
    }

    pub fn remove_pack_coauthor() -> RiseConversationEndpoint {
        Self::mutation(rise_wire_methods::stickers::SERVICE, rise_wire_methods::stickers::REMOVE_PACK_COAUTHOR, false)
    }

    pub fn leave_pack() -> RiseConversationEndpoint {
        Self::mutation(rise_wire_methods::stickers::SERVICE, rise_wire_methods::stickers::LEAVE_PACK, false)
    }

    pub fn transfer_pack_ownership() -> RiseConversationEndpoint {
        Self::mutation(rise_wire_methods::stickers::SERVICE, rise_wire_methods::stickers::TRANSFER_PACK_OWNERSHIP, false)
    }

    /// boundary is closed on purpose
    /// an unknown chat or message op in an active session must fail loudly and not leak to legacy
    pub fn endpoint(service: &str, method: &str) -> Option<&'static RiseConversationEndpoint> {
        let normalized_service = trimming_whitespaces_and_newlines(service);
        let normalized_method = trimming_whitespaces_and_newlines(method);
        Self::all().iter().find(|endpoint| {
            endpoint.service == normalized_service && endpoint.method == normalized_method
        })
    }

    pub fn all() -> &'static [RiseConversationEndpoint] {
        &ALL
    }

    fn default_timeout() -> Duration {
        Duration::from_millis(u64::from(TIMEOUT_DEFAULT_MS))
    }

    #[track_caller]
    fn read(service: &str, method: &str, timeout: Option<Duration>) -> RiseConversationEndpoint {
        RiseConversationEndpoint::canonical(service, method, RiseEndpointKind::Read, timeout, false)
    }

    #[track_caller]
    fn mutation(service: &str, method: &str, survives_suspension: bool) -> RiseConversationEndpoint {
        RiseConversationEndpoint::canonical(
            service,
            method,
            RiseEndpointKind::Mutation,
            None,
            survives_suspension,
        )
    }

    #[track_caller]
    fn request_id_mutation(service: &str, method: &str) -> RiseConversationEndpoint {
        RiseConversationEndpoint::canonical(
            service,
            method,
            RiseEndpointKind::RequestIdMutation,
            None,
            false,
        )
    }

    #[track_caller]
    fn idempotent(service: &str, method: &str) -> RiseConversationEndpoint {
        RiseConversationEndpoint::canonical(
            service,
            method,
            RiseEndpointKind::IdempotentMutation,
            None,
            false,
        )
    }
}

static ALL: LazyLock<Vec<RiseConversationEndpoint>> = LazyLock::new(|| {
    vec![
        RiseConversationEndpoints::get_link_preview(),
        RiseConversationEndpoints::get_chats(),
        RiseConversationEndpoints::get_common_chats(),
        RiseConversationEndpoints::get_chat_by_id(),
        RiseConversationEndpoints::get_archived_chats(),
        RiseConversationEndpoints::get_unread_summary(),
        RiseConversationEndpoints::get_message_requests(),
        RiseConversationEndpoints::get_chat_folders(),
        RiseConversationEndpoints::get_chat_members(),
        RiseConversationEndpoints::get_admin_members(),
        RiseConversationEndpoints::get_blocked_members(),
        RiseConversationEndpoints::get_chat_actions_history(),
        RiseConversationEndpoints::get_topics(),
        RiseConversationEndpoints::get_invite_links(),
        RiseConversationEndpoints::get_join_requests(),
        RiseConversationEndpoints::get_chat_preview_by_invite_link(),
        RiseConversationEndpoints::get_chat_settings(),
        RiseConversationEndpoints::get_linked_chat(),
        RiseConversationEndpoints::get_chat_analytics(),
        RiseConversationEndpoints::get_forum_structure(),
        RiseConversationEndpoints::get_effective_permissions(),
        RiseConversationEndpoints::get_topic_overwrites(),
        RiseConversationEndpoints::get_section_overwrites(),
        RiseConversationEndpoints::get_share_targets(),
        RiseConversationEndpoints::get_share_topics(),
        RiseConversationEndpoints::get_group_voice_channel(),
        RiseConversationEndpoints::check_tag_exist(),
        RiseConversationEndpoints::global_search_by_tag(),
        RiseConversationEndpoints::create_chat(),
        RiseConversationEndpoints::update_chat(),
        RiseConversationEndpoints::mute_chat(),
        RiseConversationEndpoints::unmute_chat(),
        RiseConversationEndpoints::update_chat_notifications(),
        RiseConversationEndpoints::leave_chat(),
        RiseConversationEndpoints::delete_chat(),
        RiseConversationEndpoints::unpin_chat(),
        RiseConversationEndpoints::reorder_pinned_chats(),
        RiseConversationEndpoints::archive_chat(),
        RiseConversationEndpoints::unarchive_chat(),
        RiseConversationEndpoints::resolve_message_request(),
        RiseConversationEndpoints::delete_all_message_requests(),
        RiseConversationEndpoints::create_chat_folder(),
        RiseConversationEndpoints::edit_chat_folder(),
        RiseConversationEndpoints::delete_chat_folder(),
        RiseConversationEndpoints::reorder_chat_folders(),
        RiseConversationEndpoints::ban_member(),
        RiseConversationEndpoints::kick_member(),
        RiseConversationEndpoints::add_member(),
        RiseConversationEndpoints::edit_member_rules(),
        RiseConversationEndpoints::edit_member(),
        RiseConversationEndpoints::transfer_ownership(),
        RiseConversationEndpoints::unban_member(),
        RiseConversationEndpoints::create_topic(),
        RiseConversationEndpoints::delete_topic(),
        RiseConversationEndpoints::update_topic(),
        RiseConversationEndpoints::update_topic_notifications(),
        RiseConversationEndpoints::create_invite_link(),
        RiseConversationEndpoints::get_voice_invite_link(),
        RiseConversationEndpoints::revoke_invite_link(),
        RiseConversationEndpoints::join_chat_by_link(),
        RiseConversationEndpoints::join_chat(),
        RiseConversationEndpoints::request_join_chat(),
        RiseConversationEndpoints::approve_join_request(),
        RiseConversationEndpoints::reject_join_request(),
        RiseConversationEndpoints::update_chat_settings(),
        RiseConversationEndpoints::link_chat(),
        RiseConversationEndpoints::unlink_chat(),
        RiseConversationEndpoints::create_role(),
        RiseConversationEndpoints::update_role(),
        RiseConversationEndpoints::delete_role(),
        RiseConversationEndpoints::reorder_roles(),
        RiseConversationEndpoints::assign_role(),
        RiseConversationEndpoints::unassign_role(),
        RiseConversationEndpoints::create_section(),
        RiseConversationEndpoints::update_section(),
        RiseConversationEndpoints::delete_section(),
        RiseConversationEndpoints::reorder_sections(),
        RiseConversationEndpoints::set_topic_overwrite(),
        RiseConversationEndpoints::set_section_overwrite(),
        RiseConversationEndpoints::remove_topic_overwrite(),
        RiseConversationEndpoints::remove_section_overwrite(),
        RiseConversationEndpoints::set_member_permissions(),
        RiseConversationEndpoints::update_group_voice_channel(),
        RiseConversationEndpoints::get_messages(),
        RiseConversationEndpoints::get_messages_around(),
        RiseConversationEndpoints::get_chat_calendar(),
        RiseConversationEndpoints::get_pinned_messages(),
        RiseConversationEndpoints::get_media_messages(),
        RiseConversationEndpoints::get_file_messages(),
        RiseConversationEndpoints::get_music_messages(),
        RiseConversationEndpoints::get_audio_messages(),
        RiseConversationEndpoints::get_gif_messages(),
        RiseConversationEndpoints::get_favorite_messages(),
        RiseConversationEndpoints::get_link_messages(),
        RiseConversationEndpoints::get_message_read_users(),
        RiseConversationEndpoints::get_poll_voters(),
        RiseConversationEndpoints::get_unread_marks(),
        RiseConversationEndpoints::search_messages(),
        RiseConversationEndpoints::search_chat_messages(),
        RiseConversationEndpoints::global_search(),
        RiseConversationEndpoints::search_posts(),
        RiseConversationEndpoints::create_message(),
        RiseConversationEndpoints::edit_message(),
        RiseConversationEndpoints::delete_messages(),
        RiseConversationEndpoints::forward_messages(),
        RiseConversationEndpoints::share_payload(),
        RiseConversationEndpoints::delete_all_messages_from_user(),
        RiseConversationEndpoints::pin_message(),
        RiseConversationEndpoints::unpin_message(),
        RiseConversationEndpoints::unpin_all_messages(),
        RiseConversationEndpoints::mark_chat_as_read(),
        RiseConversationEndpoints::mark_message_as_mentioned_read(),
        RiseConversationEndpoints::mark_reactions_as_read(),
        RiseConversationEndpoints::clear_unread_marks(),
        RiseConversationEndpoints::set_chat_unread_mark(),
        RiseConversationEndpoints::mark_media_played(),
        RiseConversationEndpoints::vote_poll(),
        RiseConversationEndpoints::mark_message_viewed(),
        RiseConversationEndpoints::typing(),
        RiseConversationEndpoints::add_reaction(),
        RiseConversationEndpoints::remove_reaction(),
        RiseConversationEndpoints::send_message_sparks(),
        RiseConversationEndpoints::get_message_sparks(),
        RiseConversationEndpoints::send_inline_result(),
        RiseConversationEndpoints::get_bot_by_tag(),
        RiseConversationEndpoints::get_default_sticker_packs(),
        RiseConversationEndpoints::get_saved_sticker_packs(),
        RiseConversationEndpoints::get_my_sticker_packs(),
        RiseConversationEndpoints::get_stickers_by_pack_id(),
        RiseConversationEndpoints::get_stickers_by_pack_link(),
        RiseConversationEndpoints::get_custom_emoji_by_ids(),
        RiseConversationEndpoints::get_sticker_favourites(),
        RiseConversationEndpoints::get_sticker_recents(),
        RiseConversationEndpoints::get_gif_favourites(),
        RiseConversationEndpoints::search_gifs(),
        RiseConversationEndpoints::add_sticker_favourite(),
        RiseConversationEndpoints::remove_sticker_favourite(),
        RiseConversationEndpoints::write_sticker_recent(),
        RiseConversationEndpoints::clear_sticker_recent(),
        RiseConversationEndpoints::add_gif_favourite(),
        RiseConversationEndpoints::remove_gif_favourite(),
        RiseConversationEndpoints::save_sticker_pack(),
        RiseConversationEndpoints::remove_saved_sticker_pack(),
        RiseConversationEndpoints::delete_sticker_pack(),
        RiseConversationEndpoints::create_sticker_pack(),
        RiseConversationEndpoints::create_sticker(),
        RiseConversationEndpoints::edit_sticker(),
        RiseConversationEndpoints::delete_sticker(),
        RiseConversationEndpoints::reorder_stickers(),
        RiseConversationEndpoints::reorder_saved_packs(),
        RiseConversationEndpoints::create_pack_invite_link(),
        RiseConversationEndpoints::edit_pack_invite_link(),
        RiseConversationEndpoints::revoke_pack_invite_link(),
        RiseConversationEndpoints::get_pack_invite_links(),
        RiseConversationEndpoints::get_pack_invite_link_joiners(),
        RiseConversationEndpoints::get_pack_invite_preview(),
        RiseConversationEndpoints::join_pack_by_link(),
        RiseConversationEndpoints::get_pack_join_requests(),
        RiseConversationEndpoints::review_pack_join_request(),
        RiseConversationEndpoints::get_pack_coauthors(),
        RiseConversationEndpoints::update_pack_coauthor_rights(),
        RiseConversationEndpoints::remove_pack_coauthor(),
        RiseConversationEndpoints::leave_pack(),
        RiseConversationEndpoints::transfer_pack_ownership(),
    ]
});

impl From<&RiseEndpoint> for RiseConversationEndpointKind {
    fn from(endpoint: &RiseEndpoint) -> Self {
        match endpoint.kind {
            RiseEndpointKind::Read => Self::Read,
            RiseEndpointKind::Mutation => Self::Mutation,
            RiseEndpointKind::RequestIdMutation => Self::RequestIDMutation,
            RiseEndpointKind::IdempotentMutation => {
                debug_assert!(
                    endpoint.idempotency_field.is_some(),
                    "{}: idempotent mutation without a dedup field",
                    endpoint.name
                );
                Self::IdempotentMutation {
                    field: endpoint.idempotency_field.unwrap_or(rise_key::chat::REQUEST_ID).to_owned(),
                }
            }
        }
    }
}

#[derive(Clone)]
pub struct RiseConversationRPC {
    pub engine: Arc<RiseEngine>,
}

impl RiseConversationRPC {
    pub fn new(engine: Arc<RiseEngine>) -> Self {
        Self { engine }
    }

    pub async fn call_method<Method: RiseWireMethod>(
        &self,
        request: &Method::Request,
        request_id: Option<RiseRequestID>,
    ) -> anyhow::Result<Method::Response> {
        self.engine
            .call::<Method>(request, request_id, RiseWireRequestMetadata::default())
            .await
    }

    /// untyped call, answer comes back without re encoding the body
    pub async fn call_body(
        &self,
        endpoint: impl Borrow<RiseConversationEndpoint> + Send,
        body: &RiseJSONValue,
        request_id: Option<RiseRequestID>,
    ) -> anyhow::Result<RiseJSONValue> {
        let descriptor = Borrow::<RiseConversationEndpoint>::borrow(&endpoint).descriptor();
        self.engine
            .call_descriptor(&descriptor, body, request_id, RiseWireRequestMetadata::default())
            .await
    }

    pub async fn attested_call<Request, Response>(
        &self,
        endpoint: impl Borrow<RiseConversationEndpoint> + Send,
        request: Request,
    ) -> anyhow::Result<Response>
    where
        Request: Serialize + Send + Sync,
        Response: DeserializeOwned + Send,
    {
        let body = serde_json::to_vec(&request)?;
        let descriptor = Borrow::<RiseConversationEndpoint>::borrow(&endpoint).descriptor();
        let headers =
            AppAttestClient::request_headers(&descriptor.service, &descriptor.method, &body).await;
        if headers.is_empty() {
            return self.call(endpoint, request).await;
        }
        let payload = self
            .engine
            .call_descriptor(
                &descriptor,
                &RiseJSONValue::String(String::from_utf8_lossy(&body).into_owned()),
                None,
                RiseWireRequestMetadata {
                    headers: headers.into_iter().collect::<BTreeMap<_, _>>(),
                    ..RiseWireRequestMetadata::default()
                },
            )
            .await?;
        payload
            .decode_as::<Response>()
            .map_err(|_| anyhow::Error::from(RiseWireError::ResponseDecodingFailed))
    }

    /// typed call, also takes a raw json value as the request and serde encodes that as itself
    pub async fn call<Request, Response>(
        &self,
        endpoint: impl Borrow<RiseConversationEndpoint> + Send,
        request: Request,
    ) -> anyhow::Result<Response>
    where
        Request: Serialize + Send + Sync,
        Response: DeserializeOwned + Send,
    {
        self.call_with_request_id(endpoint, request, None).await
    }

    pub async fn call_with_request_id<Request, Response>(
        &self,
        endpoint: impl Borrow<RiseConversationEndpoint> + Send,
        request: Request,
        request_id: Option<RiseRequestID>,
    ) -> anyhow::Result<Response>
    where
        Request: Serialize + Send + Sync,
        Response: DeserializeOwned + Send,
    {
        let descriptor = Borrow::<RiseConversationEndpoint>::borrow(&endpoint).descriptor();
        self.engine
            .call_descriptor_response::<Request, Response>(
                &descriptor,
                &request,
                request_id,
                RiseWireRequestMetadata::default(),
            )
            .await
    }
}
