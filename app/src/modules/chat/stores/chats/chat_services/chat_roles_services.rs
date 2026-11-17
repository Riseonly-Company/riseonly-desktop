use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use gpui::{App, Context};
use rise_canon::RiseEvent;
use rise_canon::rise_key::chat as key;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_events::name;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::core::lib::any_dictionary_bridge::AnyDictionaryBridge;
use crate::core::lib::sai::core::utilities::sai_cache::SaiCache;
use crate::core::lib::sai::core::utilities::sai_cache_cleaner::SaiCacheCleanLayer;
use crate::core::lib::sai::core::utilities::sai_payload_path::SaiJSONCodec;
use crate::core::lib::sai::modules::sai_updater::sai_updater::{SaiUpdater, SaiUpdaterWriteScope};
use crate::core::lib::sai::modules::sai_ws::rise_event_payload::RiseEventPayload;
use crate::core::lib::sai::modules::sai_ws::sai_ws_dropped_topic_summary_carry_over::SaiWsDroppedTopicSummaryCarryOver;
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{SaiWsCachePreferredLayer, SaiWsDiskCachePolicy};
use crate::core::lib::sai::modules::sai_ws::sai_ws_response_cache::SaiWsResponseCache;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::engine::chat_engine_hooks::ChatEngineMainCallback;
use crate::modules::chat::engine::core::rise_conversation_presentation::RiseConversationPresentation;
use crate::modules::chat::engine::forum::rise_forum_voice_presence::RiseForumVoicePresentation;
use crate::modules::chat::shared::chat_permission_resolution::ChatPermissionResolution;
use crate::modules::chat::stores::chat_guide_sandbox_hooks::chat_guide_sandbox_hooks;
use crate::modules::chat::stores::chat_stores_hooks::{chat_stores_rtc_hooks, chat_stores_user_hooks};
use crate::modules::chat::stores::chats::chat_actions::chat_actions::ChatActionsStore;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{
    ChatChannelManagementPolicy, ChatInfo, ChatMuteScopes, ChatPerm, ChatPermExt, ChatPermOverwrite, ChatRole,
    ChatSection, ChatTopic, OverwriteScope,
};
use crate::modules::chat::stores::chats::chat_actions::chat_wire_types::{
    EffectivePermissionsDTO, EffectivePermissionsDTOExt, GetOverwritesGatewayDTO, OverwriteWireDTOExt,
    RiseStoredForumStructure, RoleWireDTO, RoleWireDTOExt, SectionWireDTOExt,
};
use crate::modules::chat::stores::chats::chat_interactions::chat_interactions::ChatInteractionsStore;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::chat::stores::message::message_interactions::message_interactions::MessageInteractionsStore;

pub struct ForumStructureMutationSnapshot {
    roles: Option<Vec<Map<String, Value>>>,
}

#[derive(Clone, Debug, Default)]
pub struct ForumTopicGrouping {
    pub uncategorized: Vec<ChatTopic>,
    by_section_id: HashMap<String, Vec<ChatTopic>>,
}

impl ForumTopicGrouping {
    pub fn new(topics: &[ChatTopic]) -> Self {
        Self::grouped(topics, None)
    }

    pub fn in_sections(topics: &[ChatTopic], sections: &[ChatSection]) -> Self {
        let known: HashSet<&str> = sections.iter().map(|section| section.id.as_str()).collect();
        Self::grouped(topics, Some(&known))
    }

    fn grouped(topics: &[ChatTopic], known_sections: Option<&HashSet<&str>>) -> Self {
        let mut uncategorized = Vec::with_capacity(topics.len());
        let mut by_section_id: HashMap<String, Vec<ChatTopic>> = HashMap::new();

        for topic in topics.iter().filter(|topic| !topic.is_general) {
            let orphaned = known_sections.is_some_and(|known| !known.contains(topic.section_id.as_str()));
            if topic.section_id.is_empty() || orphaned {
                uncategorized.push(topic.clone());
            } else {
                by_section_id.entry(topic.section_id.clone()).or_default().push(topic.clone());
            }
        }

        Self { uncategorized, by_section_id }
    }

    pub fn topics(&self, section_id: &str) -> &[ChatTopic] {
        self.by_section_id.get(section_id).map_or(&[], Vec::as_slice)
    }
}

#[derive(Clone, Debug)]
pub struct ForumTopicListSnapshot {
    pub chat: ChatInfo,
    pub sections: Vec<ChatSection>,
    pub general_topic: Option<ChatTopic>,
    pub grouping: ForumTopicGrouping,
    pub permissions: i64,
    pub inherited_notification_scopes: ChatMuteScopes,
    pub global_mute_forces_muted: bool,
    pub has_cached_structure: bool,
    pub general_hidden: bool,
}

impl ForumTopicListSnapshot {
    pub fn can_manage_topics(&self) -> bool {
        ChatPerm::has(self.permissions, ChatPerm::MANAGE_TOPICS)
    }

    pub fn can_manage_voice_channels(&self) -> bool {
        ChatPerm::has(self.permissions, ChatPerm::MANAGE_VOICE_CHANNEL)
    }

    pub fn can_create_channels(&self) -> bool {
        self.can_manage_topics() || self.can_manage_voice_channels()
    }

    pub fn can_manage(&self, topic: &ChatTopic) -> bool {
        ChatChannelManagementPolicy::can_manage(topic.channel_kind, self.permissions)
    }

    pub fn can_manage_sections(&self) -> bool {
        ChatPerm::has(self.permissions, ChatPerm::MANAGE_SECTIONS)
    }

    pub fn can_manage_roles(&self) -> bool {
        ChatPerm::has(self.permissions, ChatPerm::MANAGE_ROLES)
    }

    pub fn is_effectively_muted(&self, topic: &ChatTopic) -> bool {
        if self.global_mute_forces_muted {
            return true;
        }
        let scopes = if topic.notification_inherited {
            self.inherited_notification_scopes
        } else {
            topic.notification_scopes
        };
        topic.is_muted || scopes.is_fully_muted()
    }
}

struct ForumStructureProjection {
    has_payload: bool,
    sections: Vec<ChatSection>,
    topics: Vec<ChatTopic>,
    general_topic: Option<ChatTopic>,
    roles: Vec<ChatRole>,
    general_hidden: bool,
}

impl ForumStructureProjection {
    fn empty() -> Rc<Self> {
        Rc::new(Self {
            has_payload: false,
            sections: Vec::new(),
            topics: Vec::new(),
            general_topic: None,
            roles: Vec::new(),
            general_hidden: false,
        })
    }
}

pub(super) fn removed_chat_messages_cache_key(chat_id: &str) -> String {
    SaiCache::new(["getMessages", chat_id]).key
}

pub(super) fn defer_forum_structure_refresh(chat_id: String, cx: &mut App) {
    cx.defer(move |cx| {
        ChatActionsStore::shared(cx)
            .update(cx, |actions, cx| actions.get_forum_structure_action(&chat_id, true, None, None, cx));
    });
}

fn defer_forum_access_check(chat_id: String, topics_before: HashSet<String>, cx: &mut App) {
    if topics_before.is_empty() {
        return;
    }
    cx.defer(move |cx| {
        let loaded_chat_id = chat_id.clone();
        let on_loaded: ChatEngineMainCallback = Rc::new(move |cx: &mut App| {
            ChatServicesStore::shared(cx).update(cx, |services, cx| {
                let topics_after = services.in_memory_forum_listed_topic_ids(&loaded_chat_id, cx);
                services.note_lost_forum_topics(&loaded_chat_id, &topics_before, &topics_after, cx);
            });
        });
        ChatActionsStore::shared(cx)
            .update(cx, |actions, cx| actions.get_forum_structure_action(&chat_id, true, Some(on_loaded), None, cx));
    });
}

fn defer_load_effective_permissions(chat_id: String, topic_id: Option<String>, cx: &mut App) {
    cx.defer(move |cx| {
        ChatInteractionsStore::shared(cx).update(cx, |interactions, cx| {
            interactions.load_effective_permissions(&chat_id, topic_id.as_deref(), true, cx);
        });
    });
}

fn defer_revalidate_forum_feed(chat_id: String, cx: &mut App) {
    cx.defer(move |cx| {
        MessageInteractionsStore::shared(cx)
            .update(cx, |interactions, cx| interactions.revalidate_forum_feed(&chat_id, cx));
    });
}

fn guide_owns_forum(chat_id: &str, cx: &App) -> bool {
    (chat_guide_sandbox_hooks().owns_forum_chat_id)(chat_id, cx)
}

fn guide_owns_roles(chat_id: &str, cx: &App) -> bool {
    (chat_guide_sandbox_hooks().owns_roles)(chat_id, cx)
}

fn active_room_config(cx: &mut App) -> Option<(String, Option<String>)> {
    chat_stores_rtc_hooks().and_then(|hooks| (hooks.active_room_config)(cx))
}

impl ChatServicesStore {
    pub fn forum_structure_cache_key(chat_id: &str) -> String {
        SaiCache::new(["get_forum_structure", chat_id]).key
    }

    pub fn effective_perms_cache_key(chat_id: &str, topic_id: Option<&str>) -> String {
        let Some(topic_id) = topic_id.filter(|topic_id| !topic_id.is_empty()) else {
            return SaiCache::new(["get_effective_permissions", chat_id]).key;
        };
        SaiCache::new(["get_effective_permissions", chat_id, "topic", topic_id]).key
    }

    pub fn overwrites_cache_key(chat_id: &str, scope: OverwriteScope, scope_id: &str) -> String {
        let method = if scope == OverwriteScope::Topic { "get_topic_overwrites" } else { "get_section_overwrites" };
        SaiCache::new([method, chat_id, scope_id]).key
    }

    pub fn member_override_cache_key(chat_id: &str, participant_id: &str) -> String {
        SaiCache::new(["get_member_permissions", chat_id, participant_id]).key
    }

    fn decoded_forum<T: DeserializeOwned + Clone + 'static>(&self, cache_key: &str, cx: &mut App) -> Option<T> {
        SaiWsResponseCache::observed_projection(
            cache_key,
            &format!("chat-decoded-forum-v2:{}", std::any::type_name::<T>()),
            SaiWsCachePreferredLayer::MemoryFirst,
            SaiWsDiskCachePolicy::Persistent,
            |payload: Option<&Value>| SaiJSONCodec::decode::<T>(payload),
            cx,
        )
    }

    fn forum_structure_projection(
        &self,
        chat_id: &str,
        allow_disk_hydration: bool,
        cx: &mut App,
    ) -> Rc<ForumStructureProjection> {
        if guide_owns_forum(chat_id, cx) {
            let sandbox = chat_guide_sandbox_hooks();
            return Rc::new(ForumStructureProjection {
                has_payload: true,
                sections: (sandbox.forum_sections)(cx),
                topics: (sandbox.forum_topics)(cx),
                general_topic: None,
                roles: (sandbox.forum_roles)(cx),
                general_hidden: false,
            });
        }
        let cache_key = Self::forum_structure_cache_key(chat_id);
        if !allow_disk_hydration && SaiWsResponseCache::observed_memory_only(&cache_key, cx).is_none() {
            return ForumStructureProjection::empty();
        }

        SaiWsResponseCache::observed_projection(
            &cache_key,
            "chat-forum-structure-projection-v2",
            SaiWsCachePreferredLayer::MemoryFirst,
            SaiWsDiskCachePolicy::Persistent,
            |payload: Option<&Value>| {
                let Some(dto) = SaiJSONCodec::decode::<RiseStoredForumStructure>(payload) else {
                    return ForumStructureProjection::empty();
                };

                let mut sections: Vec<ChatSection> =
                    dto.sections.unwrap_or_default().iter().map(SectionWireDTOExt::to_section).collect();
                sections.sort_by_key(|section| section.position);
                let mut topics: Vec<ChatTopic> =
                    dto.topics.unwrap_or_default().iter().map(|topic| topic.to_topic()).collect();
                topics.sort_by_key(|topic| topic.position);
                tracing::warn!(target: "riseonly::X1046", chat = chat_id, sections = sections.len(), topics = topics.len(), "forum projection rebuilt");
                let mut roles: Vec<ChatRole> =
                    dto.roles.unwrap_or_default().iter().map(RoleWireDTOExt::to_role).collect();
                roles.sort_by(|a, b| b.position.cmp(&a.position));
                let general_hidden = dto.general_topic.is_none() && dto.hide_all_topics == Some(true);
                Rc::new(ForumStructureProjection {
                    has_payload: true,
                    sections,
                    topics,
                    general_topic: dto.general_topic.as_ref().map(|topic| topic.to_topic()),
                    roles,
                    general_hidden,
                })
            },
            cx,
        )
    }

    pub fn roles(&self, chat_id: &str, cx: &mut App) -> Vec<ChatRole> {
        self.forum_structure_projection(chat_id, true, cx).roles.clone()
    }

    pub fn custom_roles(&self, chat_id: &str, cx: &mut App) -> Vec<ChatRole> {
        self.roles(chat_id, cx).into_iter().filter(|role| !role.is_default).collect()
    }

    pub fn viewer_role_rank(&self, chat_id: &str, cx: &mut App) -> Option<i64> {
        if guide_owns_roles(chat_id, cx) {
            return None;
        }
        let chat = self.chat_info(chat_id, cx)?;
        if ChatPerm::is_owner_role(chat.role.as_deref()) {
            return Some(i64::from(i32::MAX));
        }
        let held = self.effective_permissions(chat_id, None, cx)?.role_ids;
        let top = self
            .custom_roles(chat_id, cx)
            .iter()
            .filter(|role| held.contains(&role.id))
            .map(|role| role.position)
            .max()
            .unwrap_or(0);
        Some(if ChatPerm::is_owner_or_legacy_admin_role(chat.role.as_deref()) {
            top.max(i64::from(i32::MAX) - 1)
        } else {
            top
        })
    }

    pub fn viewer_outranks(&self, chat_id: &str, role: &ChatRole, cx: &mut App) -> bool {
        if role.is_default {
            return true;
        }
        let Some(rank) = self.viewer_role_rank(chat_id, cx) else {
            return true;
        };
        rank > role.position
    }

    pub fn viewer_can_grant(&self, chat_id: &str, role: &ChatRole, cx: &mut App) -> bool {
        if !self.viewer_outranks(chat_id, role, cx) {
            return false;
        }
        let Some(permissions) = self.effective_permissions(chat_id, None, cx) else {
            return true;
        };
        ChatPerm::has(permissions.mask(), role.permissions)
    }

    pub fn sections(&self, chat_id: &str, cx: &mut App) -> Vec<ChatSection> {
        self.forum_structure_projection(chat_id, true, cx).sections.clone()
    }

    pub fn forum_topics(&self, chat_id: &str, cx: &mut App) -> Vec<ChatTopic> {
        self.forum_structure_projection(chat_id, true, cx).topics.clone()
    }

    pub fn forum_manageable_topics(&self, chat_id: &str, cx: &mut App) -> Vec<ChatTopic> {
        let structure: Vec<ChatTopic> =
            self.forum_topics(chat_id, cx).into_iter().filter(|topic| !topic.is_general).collect();
        if !structure.is_empty() {
            return structure;
        }
        self.topics(chat_id, cx).into_iter().filter(|topic| !topic.is_general).collect()
    }

    pub fn in_memory_forum_topics(&self, chat_id: &str, cx: &mut App) -> Vec<ChatTopic> {
        self.forum_structure_projection(chat_id, false, cx).topics.clone()
    }

    /// the general topic sits outside the topic rows, so a forced read that dropped it never
    /// reported it lost
    /// and it stayed open beside the emptied list
    pub fn in_memory_forum_listed_topic_ids(&self, chat_id: &str, cx: &mut App) -> HashSet<String> {
        let projection = self.forum_structure_projection(chat_id, false, cx);
        let mut ids: HashSet<String> = projection.topics.iter().map(|topic| topic.id.clone()).collect();
        if let Some(general) = &projection.general_topic {
            ids.insert(general.id.clone());
        }
        if projection.has_payload && !projection.general_hidden {
            ids.insert(String::new());
        }
        ids
    }

    pub fn in_memory_forum_readable_topic_ids(&self, chat_id: &str, cx: &mut App) -> Option<HashSet<String>> {
        let projection = self.forum_structure_projection(chat_id, false, cx);
        let can_tell = !projection.topics.is_empty() || projection.general_topic.is_some() || projection.general_hidden;
        if !projection.has_payload || !can_tell {
            return None;
        }
        let mut ids: HashSet<String> = projection.topics.iter().map(|topic| topic.id.clone()).collect();
        if let Some(general) = &projection.general_topic {
            ids.insert(general.id.clone());
        }
        Some(ids)
    }

    pub fn forum_topic_previews_look_dropped(&self, chat_id: &str, cx: &mut App) -> bool {
        if SaiWsDroppedTopicSummaryCarryOver::is_carrying_over(&Self::forum_structure_cache_key(chat_id), cx) {
            return true;
        }
        if self.in_memory_forum_chat_info(chat_id, cx).and_then(|chat| chat.last_message).is_none() {
            return false;
        }
        let projection = self.forum_structure_projection(chat_id, false, cx);
        if !projection.has_payload
            || projection.general_hidden
            || projection.general_topic.as_ref().is_some_and(|topic| topic.last_message.is_some())
        {
            return false;
        }
        !projection.topics.iter().any(|topic| topic.last_message.is_some())
    }

    fn in_memory_forum_chat_info(&self, chat_id: &str, cx: &mut App) -> Option<ChatInfo> {
        let presentation = RiseConversationPresentation::shared(cx);
        let presentation = presentation.read(cx);
        if presentation.is_active()
            && let Some(chat) = presentation.chat(chat_id)
        {
            return Some(chat.clone());
        }
        if let Some(indexed) = self.chat_info_index.borrow().get(chat_id) {
            return Some(indexed.clone());
        }

        if let Some(chat) = self.get_chat_by_id_sai.data().and_then(|data| data.chat)
            && chat.id == chat_id
        {
            return Some(chat.to_chat_info());
        }
        if let Some(chat) = self
            .get_chats_sai
            .data()
            .and_then(|data| data.chats)
            .and_then(|chats| chats.into_iter().find(|chat| chat.id == chat_id))
        {
            return Some(chat.to_chat_info());
        }
        if let Some(chat) = self
            .get_archived_chats_sai
            .data()
            .and_then(|data| data.chats)
            .and_then(|chats| chats.into_iter().find(|chat| chat.id == chat_id))
        {
            return Some(chat.to_chat_info());
        }
        self
            .get_message_requests_sai
            .data()
            .and_then(|data| data.chats)
            .and_then(|chats| chats.into_iter().find(|chat| chat.id == chat_id))
            .map(|chat| chat.to_chat_info())
    }

    fn applying_live_voice_state(&self, topics: &[ChatTopic], chat_id: &str, cx: &mut App) -> Vec<ChatTopic> {
        let presence = RiseForumVoicePresentation::shared(cx);
        let Some(live) = presence.read(cx).snapshot().channels.get(chat_id).filter(|live| !live.is_empty()) else {
            return topics.to_vec();
        };
        topics
            .iter()
            .map(|topic| {
                let mut patched = topic.clone();
                let Some(state) = live.get(&topic.id).filter(|_| topic.is_rtc_channel()) else {
                    return patched;
                };
                if let Some(fetched) = patched.voice_state.as_mut()
                    && fetched.generated_at_ms > state.generated_at_ms
                {
                    fetched.visible_limit = state.visible_limit;
                    return patched;
                }
                patched.voice_state = Some(state.clone());
                patched
            })
            .collect()
    }

    pub fn forum_topic_list_snapshot(&self, chat_id: &str, cx: &mut App) -> ForumTopicListSnapshot {
        let projection = self.forum_structure_projection(chat_id, false, cx);
        let chat =
            self.in_memory_forum_chat_info(chat_id, cx).unwrap_or_else(|| ChatInfo::new(chat_id.to_owned()));
        let user_hooks = chat_stores_user_hooks();
        let mute_all_chats_enabled = user_hooks.is_some_and(|hooks| (hooks.mute_all_chats_enabled)(cx));
        let is_exception = user_hooks.is_some_and(|hooks| (hooks.chat_mute_override_ids)(cx).contains(chat_id));
        let global_mute_forces_muted = mute_all_chats_enabled && !is_exception;
        let inherited_scopes =
            Self::effective_notification_scopes(chat.notification_scopes(), mute_all_chats_enabled, is_exception);
        let permissions_cache_key = Self::effective_perms_cache_key(chat_id, None);
        let permissions = if guide_owns_forum(chat_id, cx) {
            (chat_guide_sandbox_hooks().forum_permissions)(cx).mask()
        } else if SaiWsResponseCache::observed_memory_only(&permissions_cache_key, cx).is_none() {
            0
        } else {
            self.effective_permissions(chat_id, None, cx).map_or(0, |permissions| permissions.mask())
        };
        let general_topic = projection
            .general_topic
            .clone()
            .or_else(|| projection.topics.iter().find(|topic| topic.is_general).cloned())
            .or_else(|| self.in_memory_topics_general_topic(chat_id, cx));
        let live_topics = self.applying_live_voice_state(&projection.topics, chat_id, cx);
        ForumTopicListSnapshot {
            chat,
            sections: projection.sections.clone(),
            general_topic,
            grouping: ForumTopicGrouping::in_sections(&live_topics, &projection.sections),
            permissions,
            inherited_notification_scopes: inherited_scopes,
            global_mute_forces_muted,
            has_cached_structure: projection.has_payload,
            general_hidden: projection.general_hidden,
        }
    }

    pub fn role(&self, chat_id: &str, role_id: &str, cx: &mut App) -> Option<ChatRole> {
        self.roles(chat_id, cx).into_iter().find(|role| role.id == role_id)
    }

    pub fn everyone_role(&self, chat_id: &str, cx: &mut App) -> Option<ChatRole> {
        self.roles(chat_id, cx).into_iter().find(|role| role.is_default)
    }

    pub fn can(&self, chat_id: &str, perm: i64, cx: &mut App) -> bool {
        let effective = self.effective_permissions(chat_id, None, cx);
        ChatPerm::has(effective.map_or(0, |effective| effective.mask()), perm)
    }

    pub fn effective_permissions(
        &self,
        chat_id: &str,
        topic_id: Option<&str>,
        cx: &mut App,
    ) -> Option<EffectivePermissionsDTO> {
        if guide_owns_forum(chat_id, cx) {
            return Some((chat_guide_sandbox_hooks().forum_permissions)(cx));
        }
        self.decoded_forum(&Self::effective_perms_cache_key(chat_id, topic_id), cx)
    }

    pub fn cached_overwrites(
        &self,
        chat_id: &str,
        scope: OverwriteScope,
        scope_id: &str,
        cx: &mut App,
    ) -> Option<Vec<ChatPermOverwrite>> {
        if guide_owns_roles(chat_id, cx) {
            return (chat_guide_sandbox_hooks().overwrites)(scope, scope_id, cx);
        }
        let dto: GetOverwritesGatewayDTO =
            self.decoded_forum(&Self::overwrites_cache_key(chat_id, scope, scope_id), cx)?;
        Some(dto.overwrites.iter().map(OverwriteWireDTOExt::to_overwrite).collect())
    }

    pub fn cached_member_override(&self, chat_id: &str, participant_id: &str, cx: &mut App) -> Option<(i64, i64)> {
        if guide_owns_roles(chat_id, cx)
            && let Some(invented) = (chat_guide_sandbox_hooks().member_override)(participant_id, cx)
        {
            return Some(invented);
        }
        let dto: EffectivePermissionsDTO =
            self.decoded_forum(&Self::member_override_cache_key(chat_id, participant_id), cx)?;
        Some((dto.member_allow, dto.member_deny))
    }

    pub fn has_forum_cache(&self, chat_id: &str, cx: &mut App) -> bool {
        SaiWsResponseCache::observed_payload(
            &Self::forum_structure_cache_key(chat_id),
            SaiWsCachePreferredLayer::MemoryFirst,
            SaiWsDiskCachePolicy::PreserveExisting,
            cx,
        )
        .is_some()
    }

    pub fn forum_structure_mutation_snapshot(&self, chat_id: &str, cx: &mut App) -> ForumStructureMutationSnapshot {
        let payload = SaiWsResponseCache::observed_payload(
            &Self::forum_structure_cache_key(chat_id),
            SaiWsCachePreferredLayer::MemoryFirst,
            SaiWsDiskCachePolicy::Persistent,
            cx,
        );
        let root = AnyDictionaryBridge::dictionary(payload.as_ref());
        ForumStructureMutationSnapshot {
            roles: root.map(|root| AnyDictionaryBridge::array_of_dictionaries(root.get(key::ROLES))),
        }
    }

    pub fn restore_forum_structure_mutation(
        &self,
        snapshot: &ForumStructureMutationSnapshot,
        chat_id: &str,
        cx: &mut App,
    ) {
        if let Some(roles) = &snapshot.roles {
            let rows: Vec<Value> = roles.iter().cloned().map(Value::Object).collect();
            let fields = Map::from_iter([(key::ROLES.to_owned(), Value::Array(rows))]);
            SaiUpdater::patch_root(
                &Self::forum_structure_cache_key(chat_id),
                &fields,
                SaiUpdaterWriteScope::All,
                cx,
            )
            .ok();
        }
        self.recompute_effective_permissions_local(chat_id, None, None, None, cx);
    }

    pub fn recompute_effective_permissions_local(
        &self,
        chat_id: &str,
        explicit_role_ids: Option<Vec<String>>,
        explicit_member_allow: Option<i64>,
        explicit_member_deny: Option<i64>,
        cx: &mut App,
    ) {
        let base_key = Self::effective_perms_cache_key(chat_id, None);
        let Some(current) = self.effective_permissions(chat_id, None, cx) else {
            return;
        };
        let role_ids = explicit_role_ids.unwrap_or(current.role_ids);
        let member_allow = explicit_member_allow.unwrap_or(current.member_allow);
        let member_deny = explicit_member_deny.unwrap_or(current.member_deny);
        let all_roles = self.roles(chat_id, cx);
        let everyone_role = all_roles.iter().find(|role| role.is_default);
        let everyone_role_id = everyone_role.map(|role| role.id.as_str());
        let assigned_role_ids: HashSet<String> = role_ids.iter().cloned().collect();
        let role_permissions = all_roles
            .iter()
            .filter(|role| assigned_role_ids.contains(&role.id))
            .fold(0, |permissions, role| permissions | role.permissions);
        let is_owner = ChatPerm::is_owner_role(self.chat_info(chat_id, cx).and_then(|chat| chat.role).as_deref());
        let base_permissions = ChatPermissionResolution::base_permissions(
            is_owner,
            everyone_role.map_or(0, |role| role.permissions),
            role_permissions,
            member_allow,
            member_deny,
        );
        self.write_effective_permissions(&base_key, base_permissions, &role_ids, member_allow, member_deny, cx);

        let user_id = AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire().unwrap_or_default();
        for topic in self.forum_topics(chat_id, cx) {
            let topic_key = Self::effective_perms_cache_key(chat_id, Some(&topic.id));
            let cached = SaiWsResponseCache::observed_payload(
                &topic_key,
                SaiWsCachePreferredLayer::MemoryFirst,
                SaiWsDiskCachePolicy::Persistent,
                cx,
            );
            if cached.is_none() {
                continue;
            }
            let mut scopes = Vec::with_capacity(2);
            if !topic.section_id.is_empty() {
                let rows =
                    self.cached_overwrites(chat_id, OverwriteScope::Section, &topic.section_id, cx).unwrap_or_default();
                scopes.push(ChatPermissionResolution::overwrites(
                    &rows,
                    everyone_role_id,
                    &assigned_role_ids,
                    &user_id,
                ));
            }
            let rows = self.cached_overwrites(chat_id, OverwriteScope::Topic, &topic.id, cx).unwrap_or_default();
            scopes.push(ChatPermissionResolution::overwrites(&rows, everyone_role_id, &assigned_role_ids, &user_id));
            let permissions = ChatPermissionResolution::channel_permissions(base_permissions, is_owner, &scopes);
            self.write_effective_permissions(&topic_key, permissions, &role_ids, member_allow, member_deny, cx);
        }
    }

    fn write_effective_permissions(
        &self,
        cache_key: &str,
        permissions: i64,
        role_ids: &[String],
        member_allow: i64,
        member_deny: i64,
        cx: &mut App,
    ) {
        let dto = EffectivePermissionsDTO { member_allow, member_deny, permissions, role_ids: role_ids.to_vec() };
        let Some(root) = SaiJSONCodec::encode(&dto) else {
            return;
        };
        SaiUpdater::replace_root(cache_key, root, SaiUpdaterWriteScope::All, cx).ok();
    }

    fn role_row(&self, role: &ChatRole) -> Map<String, Value> {
        Map::from_iter([
            (key::ROLE_ID.to_owned(), Value::from(role.id.as_str())),
            (key::NAME.to_owned(), Value::from(role.name.as_str())),
            (key::COLOR.to_owned(), Value::from(role.color)),
            (key::POSITION.to_owned(), Value::from(role.position)),
            (key::PERMISSIONS.to_owned(), Value::from(role.permissions)),
            (key::MENTIONABLE.to_owned(), Value::from(role.mentionable)),
            (key::HOIST.to_owned(), Value::from(role.hoist)),
            (key::IS_DEFAULT.to_owned(), Value::from(role.is_default)),
        ])
    }

    pub fn upsert_role_cache(&self, role: ChatRole, chat_id: &str, cx: &mut App) {
        if !self.has_forum_cache(chat_id, cx) {
            return;
        }
        let cache_key = Self::forum_structure_cache_key(chat_id);
        SaiUpdater::remove_row(&cache_key, key::ROLES, key::ROLE_ID, &role.id, SaiUpdaterWriteScope::All, cx).ok();
        SaiUpdater::insert_row_if_missing(
            &cache_key,
            key::ROLES,
            key::ROLE_ID,
            self.role_row(&role),
            true,
            SaiUpdaterWriteScope::All,
            cx,
        )
        .ok();
    }

    pub fn remove_role_cache(&self, role_id: &str, chat_id: &str, cx: &mut App) {
        if !self.has_forum_cache(chat_id, cx) {
            return;
        }
        SaiUpdater::remove_row(
            &Self::forum_structure_cache_key(chat_id),
            key::ROLES,
            key::ROLE_ID,
            role_id,
            SaiUpdaterWriteScope::All,
            cx,
        )
        .ok();
    }

    pub fn reorder_roles_cache(&self, chat_id: &str, role_ids: &[String], cx: &mut App) {
        if !self.has_forum_cache(chat_id, cx) {
            return;
        }
        let cache_key = Self::forum_structure_cache_key(chat_id);
        let count = role_ids.len();
        for (index, role_id) in role_ids.iter().enumerate() {
            let patch = Map::from_iter([(key::POSITION.to_owned(), Value::from(count - index))]);
            SaiUpdater::patch_row(&cache_key, key::ROLES, key::ROLE_ID, role_id, &patch, SaiUpdaterWriteScope::All, cx)
                .ok();
        }
    }

    fn section_row(&self, section: &ChatSection) -> Map<String, Value> {
        Map::from_iter([
            (key::SECTION_ID.to_owned(), Value::from(section.id.as_str())),
            (key::NAME.to_owned(), Value::from(section.name.as_str())),
            (key::NAME_ENTITIES.to_owned(), Value::from(section.name_entities.clone().unwrap_or_default())),
            (key::POSITION.to_owned(), Value::from(section.position)),
        ])
    }

    pub fn insert_section_cache(&self, section: ChatSection, chat_id: &str, cx: &mut App) {
        if !self.has_forum_cache(chat_id, cx) {
            defer_forum_structure_refresh(chat_id.to_owned(), cx);
            return;
        }
        SaiUpdater::insert_row_if_missing(
            &Self::forum_structure_cache_key(chat_id),
            key::SECTIONS,
            key::SECTION_ID,
            self.section_row(&section),
            true,
            SaiUpdaterWriteScope::All,
            cx,
        )
        .ok();
    }

    pub fn rename_section_cache(
        &self,
        section_id: &str,
        name: &str,
        name_entities: Option<&str>,
        chat_id: &str,
        cx: &mut App,
    ) {
        if !self.has_forum_cache(chat_id, cx) {
            return;
        }
        let patch = Map::from_iter([
            (key::NAME.to_owned(), Value::from(name)),
            (key::NAME_ENTITIES.to_owned(), Value::from(name_entities.unwrap_or_default())),
        ]);
        SaiUpdater::patch_row(
            &Self::forum_structure_cache_key(chat_id),
            key::SECTIONS,
            key::SECTION_ID,
            section_id,
            &patch,
            SaiUpdaterWriteScope::All,
            cx,
        )
        .ok();
    }

    pub fn reorder_sections_cache(&self, chat_id: &str, section_ids: &[String], cx: &mut App) {
        if !self.has_forum_cache(chat_id, cx) {
            return;
        }
        let cache_key = Self::forum_structure_cache_key(chat_id);
        for (index, section_id) in section_ids.iter().enumerate() {
            let patch = Map::from_iter([(key::POSITION.to_owned(), Value::from(index))]);
            SaiUpdater::patch_row(
                &cache_key,
                key::SECTIONS,
                key::SECTION_ID,
                section_id,
                &patch,
                SaiUpdaterWriteScope::All,
                cx,
            )
            .ok();
        }
    }

    pub fn remove_section_cache(&self, section_id: &str, chat_id: &str, cx: &mut App) {
        if !self.has_forum_cache(chat_id, cx) {
            return;
        }
        SaiUpdater::remove_row(
            &Self::forum_structure_cache_key(chat_id),
            key::SECTIONS,
            key::SECTION_ID,
            section_id,
            SaiUpdaterWriteScope::All,
            cx,
        )
        .ok();
    }

    fn replace_overwrites_cache(
        &self,
        chat_id: &str,
        scope: OverwriteScope,
        scope_id: &str,
        overwrites: &[ChatPermOverwrite],
        cx: &mut App,
    ) {
        let rows: Vec<Value> = overwrites.iter().map(|overwrite| Value::Object(self.overwrite_row(overwrite))).collect();
        let root = Map::from_iter([(key::OVERWRITES.to_owned(), Value::Array(rows))]);
        SaiUpdater::replace_root(
            &Self::overwrites_cache_key(chat_id, scope, scope_id),
            root,
            SaiUpdaterWriteScope::All,
            cx,
        )
        .ok();
    }

    pub fn upsert_overwrite_cache(
        &self,
        chat_id: &str,
        scope: OverwriteScope,
        scope_id: &str,
        overwrite: ChatPermOverwrite,
        cx: &mut App,
    ) {
        let Some(mut overwrites) = self.cached_overwrites(chat_id, scope, scope_id, cx) else {
            return;
        };
        overwrites.retain(|row| row.target_type != overwrite.target_type || row.target_id != overwrite.target_id);
        overwrites.push(overwrite);
        self.replace_overwrites_cache(chat_id, scope, scope_id, &overwrites, cx);
    }

    pub fn remove_overwrite_cache(
        &self,
        chat_id: &str,
        scope: OverwriteScope,
        scope_id: &str,
        target_type: &str,
        target_id: &str,
        cx: &mut App,
    ) {
        let Some(mut overwrites) = self.cached_overwrites(chat_id, scope, scope_id, cx) else {
            return;
        };
        overwrites.retain(|row| row.target_type != target_type || row.target_id != target_id);
        self.replace_overwrites_cache(chat_id, scope, scope_id, &overwrites, cx);
    }

    pub fn upsert_member_override_cache(
        &self,
        chat_id: &str,
        participant_id: &str,
        allow: i64,
        deny: i64,
        cx: &mut App,
    ) {
        let fields = Map::from_iter([
            (key::MEMBER_ALLOW.to_owned(), Value::from(allow)),
            (key::MEMBER_DENY.to_owned(), Value::from(deny)),
        ]);
        SaiUpdater::patch_root(
            &Self::member_override_cache_key(chat_id, participant_id),
            &fields,
            SaiUpdaterWriteScope::All,
            cx,
        )
        .ok();
    }

    pub fn invalidate_effective_permissions(&self, chat_id: &str, cx: &mut App) {
        SaiWsResponseCache::remove(&Self::effective_perms_cache_key(chat_id, None), SaiCacheCleanLayer::Everywhere, cx);
        for topic in self.forum_topics(chat_id, cx) {
            SaiWsResponseCache::remove(
                &Self::effective_perms_cache_key(chat_id, Some(&topic.id)),
                SaiCacheCleanLayer::Everywhere,
                cx,
            );
        }
    }

    pub fn invalidate_effective_permissions_scope(
        &self,
        chat_id: &str,
        scope: OverwriteScope,
        scope_id: &str,
        cx: &mut App,
    ) {
        match scope {
            OverwriteScope::Topic => {
                SaiWsResponseCache::remove(
                    &Self::effective_perms_cache_key(chat_id, Some(scope_id)),
                    SaiCacheCleanLayer::Everywhere,
                    cx,
                );
            }
            OverwriteScope::Section => {
                for topic in self.forum_topics(chat_id, cx).iter().filter(|topic| topic.section_id == scope_id) {
                    SaiWsResponseCache::remove(
                        &Self::effective_perms_cache_key(chat_id, Some(&topic.id)),
                        SaiCacheCleanLayer::Everywhere,
                        cx,
                    );
                }
            }
        }
    }

    pub fn invalidate_overwrite_caches(&self, chat_id: &str, cx: &mut App) {
        for topic in self.forum_topics(chat_id, cx) {
            SaiWsResponseCache::remove(
                &Self::overwrites_cache_key(chat_id, OverwriteScope::Topic, &topic.id),
                SaiCacheCleanLayer::Everywhere,
                cx,
            );
        }
        for section in self.sections(chat_id, cx) {
            SaiWsResponseCache::remove(
                &Self::overwrites_cache_key(chat_id, OverwriteScope::Section, &section.id),
                SaiCacheCleanLayer::Everywhere,
                cx,
            );
        }
    }

    fn overwrite_row(&self, overwrite: &ChatPermOverwrite) -> Map<String, Value> {
        Map::from_iter([
            (key::TARGET_TYPE.to_owned(), Value::from(overwrite.target_type.as_str())),
            (key::TARGET_ID.to_owned(), Value::from(overwrite.target_id.as_str())),
            (key::ALLOW.to_owned(), Value::from(overwrite.allow)),
            (key::DENY.to_owned(), Value::from(overwrite.deny)),
        ])
    }

    pub fn apply_role_created_or_updated(&mut self, any: Option<&Value>, name: &str, cx: &mut Context<Self>) {
        let (chat_id, card) = match RiseEvent::decode_payload(name, any) {
            Some(RiseEvent::RoleCreated(event)) => (event.chat_id, event.role),
            Some(RiseEvent::RoleUpdated(event)) => (event.chat_id, event.role),
            _ => return,
        };
        let id = card.role_id.clone();
        if id.is_empty() {
            return;
        }
        let Some(role_dict) = SaiJSONCodec::encode(&RoleWireDTO::from_canon(&card)) else {
            return;
        };

        if self.has_forum_cache(&chat_id, cx) {
            let cache_key = Self::forum_structure_cache_key(&chat_id);
            SaiUpdater::remove_row(&cache_key, key::ROLES, key::ROLE_ID, &id, SaiUpdaterWriteScope::All, cx).ok();
            SaiUpdater::insert_row_if_missing(
                &cache_key,
                key::ROLES,
                key::ROLE_ID,
                role_dict,
                true,
                SaiUpdaterWriteScope::All,
                cx,
            )
            .ok();
        }
        self.recompute_effective_permissions_local(&chat_id, None, None, None, cx);
        self.refresh_open_surfaces_after_rights_change(&chat_id, cx);
    }

    pub fn apply_role_deleted(&mut self, any: Option<&Value>, cx: &mut Context<Self>) {
        let Some(RiseEvent::RoleDeleted(event)) = RiseEvent::decode_payload(name::ROLE_DELETED, any) else {
            return;
        };
        self.remove_role_cache(&event.role_id, &event.chat_id, cx);
        self.recompute_effective_permissions_local(&event.chat_id, None, None, None, cx);
        self.refresh_open_surfaces_after_rights_change(&event.chat_id, cx);
    }

    fn refresh_open_surfaces_after_rights_change(&self, chat_id: &str, cx: &mut App) {
        self.reload_active_room_permissions(chat_id, cx);
        defer_revalidate_forum_feed(chat_id.to_owned(), cx);
    }

    fn reload_active_room_permissions(&self, chat_id: &str, cx: &mut App) {
        let Some((config_chat_id, channel_id)) = active_room_config(cx) else {
            return;
        };
        if config_chat_id != chat_id {
            return;
        }
        let topic_id = channel_id
            .as_deref()
            .map(trimming_whitespaces_and_newlines)
            .filter(|topic_id| !topic_id.is_empty())
            .map(str::to_owned);
        defer_load_effective_permissions(chat_id.to_owned(), topic_id, cx);
    }

    pub fn apply_roles_reordered(&mut self, any: Option<&Value>, cx: &mut Context<Self>) {
        let Some(RiseEvent::RolesReordered(event)) = RiseEvent::decode_payload(name::ROLES_REORDERED, any) else {
            return;
        };
        if !event.role_ids.is_empty() {
            self.reorder_roles_cache(&event.chat_id, &event.role_ids, cx);
        } else {
            defer_forum_structure_refresh(event.chat_id, cx);
        }
    }

    pub fn apply_member_roles_changed(&mut self, any: Option<&Value>, cx: &mut Context<Self>) {
        let Some(RiseEvent::MemberRolesChanged(event)) = RiseEvent::decode_payload(name::MEMBER_ROLES_CHANGED, any)
        else {
            return;
        };
        let chat_id = event.chat_id;
        let member_id = event.user_id;
        let role_ids = event.role_ids;
        self.apply_member_role_change(&chat_id, &member_id, role_ids.clone(), event.display_color, cx);

        let effective_user_id = AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire();
        if effective_user_id.as_deref() == Some(member_id.as_str()) {
            let topics_before = self.in_memory_forum_listed_topic_ids(&chat_id, cx);
            self.recompute_effective_permissions_local(&chat_id, Some(role_ids), None, None, cx);
            self.reconcile_slow_mode(&chat_id, cx);
            self.refresh_open_surfaces_after_rights_change(&chat_id, cx);
            defer_forum_access_check(chat_id, topics_before, cx);
        }
    }

    pub fn apply_permission_overwrite_changed(
        &mut self,
        any: Option<&Value>,
        scope: OverwriteScope,
        cx: &mut Context<Self>,
    ) {
        let (chat_id, scope_id) = match scope {
            OverwriteScope::Topic => {
                let Some(RiseEvent::TopicOverwriteChanged(event)) =
                    RiseEvent::decode_payload(name::TOPIC_OVERWRITE_CHANGED, any)
                else {
                    return;
                };
                (event.chat_id, event.topic_id)
            }
            OverwriteScope::Section => {
                let Some(RiseEvent::SectionOverwriteChanged(event)) =
                    RiseEvent::decode_payload(name::SECTION_OVERWRITE_CHANGED, any)
                else {
                    return;
                };
                (event.chat_id, event.section_id)
            }
        };
        if chat_id.is_empty() || scope_id.is_empty() {
            return;
        }
        defer_revalidate_forum_feed(chat_id.clone(), cx);

        let had_scoped_permission_cache = scope == OverwriteScope::Topic
            && SaiWsResponseCache::observed_payload(
                &Self::effective_perms_cache_key(&chat_id, Some(&scope_id)),
                SaiWsCachePreferredLayer::MemoryFirst,
                SaiWsDiskCachePolicy::Persistent,
                cx,
            )
            .is_some();

        SaiWsResponseCache::remove(
            &Self::overwrites_cache_key(&chat_id, scope, &scope_id),
            SaiCacheCleanLayer::Everywhere,
            cx,
        );
        self.invalidate_effective_permissions_scope(&chat_id, scope, &scope_id, cx);

        let active_topic_id =
            active_room_config(cx).filter(|(config_chat_id, _)| *config_chat_id == chat_id).and_then(|config| config.1);

        match scope {
            OverwriteScope::Topic => {
                if !had_scoped_permission_cache && active_topic_id.as_deref() != Some(scope_id.as_str()) {
                    return;
                }
                defer_load_effective_permissions(chat_id, Some(scope_id), cx);
            }
            OverwriteScope::Section => {
                let Some(active_topic_id) = active_topic_id.filter(|topic_id| !topic_id.is_empty()) else {
                    return;
                };
                if self
                    .topic_from_any_cache(&chat_id, &active_topic_id, cx)
                    .is_some_and(|active_topic| active_topic.section_id != scope_id)
                {
                    return;
                }
                self.invalidate_effective_permissions_scope(&chat_id, OverwriteScope::Topic, &active_topic_id, cx);
                defer_load_effective_permissions(chat_id, Some(active_topic_id), cx);
            }
        }
    }

    pub fn apply_section_event(&mut self, any: Option<&Value>, name: &str, cx: &mut Context<Self>) {
        let chat_id = match RiseEvent::decode_payload(name, any) {
            Some(RiseEvent::SectionsReordered(event)) => {
                if !event.section_ids.is_empty() {
                    self.reorder_sections_cache(&event.chat_id, &event.section_ids, cx);
                    return;
                }
                event.chat_id
            }
            Some(RiseEvent::SectionCreated(event)) => event.chat_id,
            Some(RiseEvent::SectionUpdated(event)) => event.chat_id,
            Some(RiseEvent::SectionDeleted(event)) => event.chat_id,
            _ => return,
        };
        defer_forum_structure_refresh(chat_id, cx);
    }
}
