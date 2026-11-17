use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::Duration;

use gpui::{App, Context};
use rise_canon::{RiseLimits, RiseRpcCanon};
use rise_canon::rise_key::chat as key;
use rise_canon::rise_key::message as message_key;
use rise_chat::BanType;
use rise_core::foundation::foundation_date::{Date, RiseClock};
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use serde_json::{Map, Value};

use crate::core::lib::any_dictionary_bridge::AnyDictionaryBridge;
use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::sai::core::utilities::sai_cache::SaiCache;
use crate::core::lib::sai::core::utilities::sai_cache_cleaner::SaiCacheCleanLayer;
use crate::core::lib::sai::core::utilities::sai_payload_path::SaiJSONCodec;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiWsSliceStatus;
use crate::core::lib::sai::modules::sai_debouncer::sai_debouncer::SaiDebouncer;
use crate::core::lib::sai::modules::sai_updater::sai_updater::{SaiUpdater, SaiUpdaterWriteScope};
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{SaiWsCachePreferredLayer, SaiWsDiskCachePolicy};
use crate::core::lib::sai::modules::sai_ws::sai_ws_response_cache::SaiWsResponseCache;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::engine::rise_conversation_domain::RiseConversationDomainRegistry;
use crate::modules::guides::shared::guide_sandbox::GuideSandbox;
use crate::modules::chat::stores::chats::chat_actions::chat_actions::ChatActionsStore;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{
    BlockedMember, ChatAction, ChatMember, ChatMemberWireDTOExt, ChatPerm, ChatPermExt,
};
use crate::modules::chat::stores::chats::chat_actions::chat_wire_types::{
    BlockedMemberWireDTOExt, ChatActionWireDTOExt, GetBlockedMembersGatewayDTO, GetChatActionsGatewayDTO,
    GetChatMembersGatewayDTO,
};
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;

#[derive(Clone, Debug)]
pub struct ChatMemberMutationSnapshot {
    roots_by_cache_key: HashMap<String, Map<String, Value>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChatMemberListLoadState {
    Loading,
    Loaded,
    Failed,
    Refused,
}

#[derive(Default)]
struct ChatMembersProjection {
    members: Vec<ChatMember>,
    members_by_user_id: HashMap<String, ChatMember>,
    has_more: bool,
    last_id: Option<String>,
}

impl ChatMembersProjection {
    fn empty() -> Self {
        Self::default()
    }
}

fn guide_forum_members(scope: &str, cx: &App) -> Option<Vec<ChatMember>> {
    GuideSandbox::existing(cx).and_then(|sandbox| sandbox.read(cx).forum_members(scope, cx))
}

fn row_user_id(row: &Map<String, Value>) -> Option<&str> {
    row.get(key::USER_ID).and_then(Value::as_str)
}

impl ChatServicesStore {
    pub fn register_member_scope(&mut self, chat_id: &str, scope: &str, cx: &mut Context<Self>) {
        if self.member_scopes_by_chat.entry(chat_id.to_owned()).or_default().insert(scope.to_owned()) {
            cx.notify();
        }
    }

    pub fn known_member_scopes(&mut self, chat_id: &str, cx: &mut Context<Self>) -> HashSet<String> {
        let mut scopes = self.member_scopes_by_chat.get(chat_id).cloned().unwrap_or_default();
        scopes.insert(Self::member_scope_key(chat_id, None));
        for topic in self.topics(chat_id, cx) {
            scopes.insert(Self::member_scope_key(chat_id, Some(topic.id.as_str())));
        }
        scopes
    }

    fn cached_members_projection(&self, scope: &str, cx: &mut App) -> Rc<ChatMembersProjection> {
        SaiWsResponseCache::observed_projection(
            &Self::members_cache_key(scope),
            "chat-members-projection-v1",
            SaiWsCachePreferredLayer::MemoryFirst,
            SaiWsDiskCachePolicy::PreserveExisting,
            |payload| {
                let Some(dto) = SaiJSONCodec::decode::<GetChatMembersGatewayDTO>(payload) else {
                    return Rc::new(ChatMembersProjection::empty());
                };
                let members: Vec<ChatMember> = dto.members.iter().map(|member| member.to_chat_member()).collect();
                let mut members_by_user_id = HashMap::with_capacity(members.len());
                for member in &members {
                    members_by_user_id.insert(member.user_id.clone(), member.clone());
                }
                Rc::new(ChatMembersProjection {
                    members,
                    members_by_user_id,
                    has_more: dto.pagination.as_ref().is_some_and(|pagination| pagination.has_more),
                    last_id: dto.pagination.as_ref().and_then(|pagination| pagination.last_id.clone()),
                })
            },
            cx,
        )
    }

    pub fn members(&self, scope: &str, cx: &mut App) -> Vec<ChatMember> {
        if let Some(invented) = guide_forum_members(scope, cx) {
            return invented;
        }
        self.cached_members_projection(scope, cx).members.clone()
    }

    pub fn members_in_memory(&self, scope: &str, cx: &mut App) -> Vec<ChatMember> {
        if let Some(invented) = guide_forum_members(scope, cx) {
            return invented;
        }
        if SaiWsResponseCache::observed_memory_only(&Self::members_cache_key(scope), cx).is_none() {
            return Vec::new();
        }
        self.cached_members_projection(scope, cx).members.clone()
    }

    pub fn member(&self, scope: &str, user_id: &str, cx: &mut App) -> Option<ChatMember> {
        if let Some(invented) = guide_forum_members(scope, cx) {
            return invented.into_iter().find(|member| member.user_id == user_id);
        }
        self.cached_members_projection(scope, cx).members_by_user_id.get(user_id).cloned()
    }

    pub fn members_has_more(&self, scope: &str, cx: &mut App) -> bool {
        if guide_forum_members(scope, cx).is_some() {
            return false;
        }
        self.cached_members_projection(scope, cx).has_more
    }

    pub fn members_last_id(&self, scope: &str, cx: &mut App) -> Option<String> {
        if guide_forum_members(scope, cx).is_some() {
            return None;
        }
        self.cached_members_projection(scope, cx).last_id.clone()
    }

    pub fn clear_members(&mut self, scope: &str, cx: &mut Context<Self>) {
        SaiWsResponseCache::remove(&Self::members_cache_key(scope), SaiCacheCleanLayer::Memory, cx);
        cx.notify();
    }

    pub fn is_member_list_loading(&self, scope: &str) -> bool {
        self.member_list_load_states.get(scope) == Some(&ChatMemberListLoadState::Loading)
    }

    pub fn is_member_list_load_abandoned(&self, scope: &str) -> bool {
        self.member_list_load_started_at.get(scope).is_some_and(|started_at| {
            RiseClock::monotonic().saturating_sub(*started_at)
                >= Duration::from_millis(RiseRpcCanon::TIMEOUTS_DEFAULT_MS as u64)
        })
    }

    pub fn begin_member_list_load(&mut self, scope: &str, cx: &mut Context<Self>) {
        self.member_list_load_states.insert(scope.to_owned(), ChatMemberListLoadState::Loading);
        self.member_list_load_started_at.insert(scope.to_owned(), RiseClock::monotonic());
        cx.notify();
    }

    pub fn finish_member_list_load(&mut self, scope: &str, cx: &mut Context<Self>) {
        tracing::warn!(target: "riseonly::X2107", scope = %scope, "member list load finished");
        self.member_list_load_started_at.remove(scope);
        if self.member_list_load_states.insert(scope.to_owned(), ChatMemberListLoadState::Loaded)
            != Some(ChatMemberListLoadState::Loaded)
        {
            cx.notify();
        }
    }

    pub fn fail_member_list_load(&mut self, scope: &str, is_first_page: bool, refused: bool, cx: &mut Context<Self>) {
        if !is_first_page && !self.is_member_list_loading(scope) {
            return;
        }
        self.member_list_load_started_at.remove(scope);
        let state = if refused { ChatMemberListLoadState::Refused } else { ChatMemberListLoadState::Failed };
        self.member_list_load_states.insert(scope.to_owned(), state);
        cx.notify();
    }

    pub fn is_member_list_refused(&self, scope: &str) -> bool {
        self.member_list_load_states.get(scope) == Some(&ChatMemberListLoadState::Refused)
    }

    pub fn member_list_render_status(&self, scope: &str, has_members: bool) -> SaiWsSliceStatus {
        if has_members {
            return SaiWsSliceStatus::Fulfilled;
        }
        match self.member_list_load_states.get(scope) {
            Some(ChatMemberListLoadState::Loaded | ChatMemberListLoadState::Refused) => SaiWsSliceStatus::Fulfilled,
            Some(ChatMemberListLoadState::Failed) => SaiWsSliceStatus::Rejected,
            Some(ChatMemberListLoadState::Loading) | None => SaiWsSliceStatus::Pending,
        }
    }

    pub fn refresh_loaded_member_lists(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        let root_scope = Self::member_scope_key(chat_id, None);
        let loaded_scopes = self.member_scopes_by_chat.get(chat_id).cloned().unwrap_or_default();
        let mut topic_ids_by_scope: HashMap<String, String> = HashMap::new();
        for topic in self.topics(chat_id, cx) {
            topic_ids_by_scope
                .entry(Self::member_scope_key(chat_id, Some(topic.id.as_str())))
                .or_insert_with(|| topic.id.clone());
        }
        let mut stale_scopes: HashSet<String> = HashSet::from([root_scope.clone()]);
        stale_scopes.extend(loaded_scopes.iter().cloned());
        stale_scopes.extend(topic_ids_by_scope.keys().cloned());
        for scope in stale_scopes {
            let cache_key = Self::members_cache_key(&scope);
            if loaded_scopes.contains(&scope) && SaiWsResponseCache::observed_memory_only(&cache_key, cx).is_some() {
                continue;
            }
            SaiWsResponseCache::remove(&cache_key, SaiCacheCleanLayer::Everywhere, cx);
        }
        let scopes = loaded_scopes;
        if scopes.is_empty() {
            tracing::warn!(target: "riseonly::X2107", chat_id = %chat_id, "no loaded member scope on join");
            return;
        }
        for scope in &scopes {
            if SaiWsResponseCache::observed_memory_only(&Self::members_cache_key(scope), cx).is_none() {
                continue;
            }
            let loaded = self.members(scope, cx).len();
            tracing::warn!(target: "riseonly::X2107", scope = %scope, loaded, "member list refetch on join");
            if loaded as i64 > Self::MEMBER_PAGE_LIMIT {
                continue;
            }
            let topic_id = if *scope == root_scope {
                None
            } else if let Some(topic_id) = topic_ids_by_scope.get(scope) {
                Some(topic_id.clone())
            } else {
                continue;
            };
            let chat_id = chat_id.to_owned();
            cx.defer(move |cx| {
                ChatActionsStore::shared(cx).update(cx, |actions, cx| {
                    actions.get_chat_members_action(&chat_id, topic_id.as_deref(), None, false, true, None, cx);
                });
            });
        }
    }

    pub fn member_mutation_snapshot(
        &mut self,
        chat_id: &str,
        include_blocked: bool,
        cx: &mut Context<Self>,
    ) -> ChatMemberMutationSnapshot {
        let mut roots = HashMap::new();
        for scope in self.known_member_scopes(chat_id, cx) {
            self.capture_member_cache_root(&Self::members_cache_key(&scope), &mut roots, cx);
        }
        self.capture_member_cache_root(&Self::admins_cache_key(chat_id), &mut roots, cx);
        if include_blocked {
            self.capture_member_cache_root(&Self::blocked_cache_key(chat_id), &mut roots, cx);
        }
        ChatMemberMutationSnapshot { roots_by_cache_key: roots }
    }

    pub fn restore_member_mutation_snapshot(
        &mut self,
        snapshot: &ChatMemberMutationSnapshot,
        chat_id: &str,
        cx: &mut Context<Self>,
    ) {
        for (cache_key, root) in &snapshot.roots_by_cache_key {
            let _ = SaiUpdater::replace_root(cache_key, root.clone(), SaiUpdaterWriteScope::All, cx);
        }
        self.mirror_member_caches_to_rise(chat_id, cx);
        cx.notify();
    }

    pub fn apply_member_fields_local(
        &mut self,
        chat_id: &str,
        user_id: &str,
        fields: Map<String, Value>,
        cx: &mut Context<Self>,
    ) {
        if chat_id.is_empty() || user_id.is_empty() || fields.is_empty() {
            return;
        }
        let source_row = self.raw_member_row(chat_id, user_id, cx);

        for scope in self.known_member_scopes(chat_id, cx) {
            self.patch_member_row(&Self::members_cache_key(&scope), user_id, &fields, cx);
        }

        let admins_key = Self::admins_cache_key(chat_id);
        if let Some(role) = fields.get(key::ROLE).and_then(Value::as_str) {
            if Self::is_admin_member_role(role) {
                let was_admin = self.member_cache_contains(&admins_key, user_id, cx);
                if let Some(mut source_row) = source_row {
                    for (field, value) in &fields {
                        source_row.insert(field.clone(), value.clone());
                    }
                    let _ = SaiUpdater::insert_row_if_missing(
                        &admins_key,
                        key::MEMBERS,
                        message_key::USER_ID,
                        source_row,
                        true,
                        SaiUpdaterWriteScope::All,
                        cx,
                    );
                    if !was_admin {
                        self.adjust_member_cache_total(&admins_key, 1, cx);
                    }
                }
                self.patch_member_row(&admins_key, user_id, &fields, cx);
            } else {
                let was_admin = self.member_cache_contains(&admins_key, user_id, cx);
                let _ = SaiUpdater::remove_row(
                    &admins_key,
                    key::MEMBERS,
                    message_key::USER_ID,
                    user_id,
                    SaiUpdaterWriteScope::All,
                    cx,
                );
                if was_admin {
                    self.adjust_member_cache_total(&admins_key, -1, cx);
                }
            }
        } else {
            self.patch_member_row(&admins_key, user_id, &fields, cx);
        }

        self.mirror_member_caches_to_rise(chat_id, cx);
        cx.notify();
    }

    pub fn remove_member_local(
        &mut self,
        chat_id: &str,
        user_id: &str,
        reason: Option<&str>,
        add_to_blocked: bool,
        cx: &mut Context<Self>,
    ) {
        if chat_id.is_empty() || user_id.is_empty() {
            return;
        }
        let source_row = self.raw_member_row(chat_id, user_id, cx);

        for scope in self.known_member_scopes(chat_id, cx) {
            let cache_key = Self::members_cache_key(&scope);
            let contained = self.member_cache_contains(&cache_key, user_id, cx);
            let _ = SaiUpdater::remove_row(
                &cache_key,
                key::MEMBERS,
                message_key::USER_ID,
                user_id,
                SaiUpdaterWriteScope::All,
                cx,
            );
            if contained {
                self.adjust_member_cache_total(&cache_key, -1, cx);
            }
        }
        let admins_key = Self::admins_cache_key(chat_id);
        let was_admin = self.member_cache_contains(&admins_key, user_id, cx);
        let _ = SaiUpdater::remove_row(
            &admins_key,
            key::MEMBERS,
            message_key::USER_ID,
            user_id,
            SaiUpdaterWriteScope::All,
            cx,
        );
        if was_admin {
            self.adjust_member_cache_total(&admins_key, -1, cx);
        }

        let blocked_key = Self::blocked_cache_key(chat_id);
        if add_to_blocked
            && self.member_cache_root(&blocked_key, cx).is_some()
            && let Some(source_row) = source_row
        {
            let was_blocked = self.member_cache_contains(&blocked_key, user_id, cx);
            let blocked_row = self.make_blocked_row(&source_row, chat_id, reason, cx);
            let _ = SaiUpdater::insert_row_if_missing(
                &blocked_key,
                key::MEMBERS,
                message_key::USER_ID,
                blocked_row,
                true,
                SaiUpdaterWriteScope::All,
                cx,
            );
            if !was_blocked {
                self.adjust_member_cache_total(&blocked_key, 1, cx);
            }
        }

        self.mirror_member_caches_to_rise(chat_id, cx);
        cx.notify();
    }

    fn is_admin_member_role(role: &str) -> bool {
        ChatPerm::is_owner_or_legacy_admin_role(Some(role))
    }

    fn patch_member_row(&self, cache_key: &str, user_id: &str, fields: &Map<String, Value>, cx: &mut App) {
        let _ = SaiUpdater::patch_row(
            cache_key,
            key::MEMBERS,
            message_key::USER_ID,
            user_id,
            fields,
            SaiUpdaterWriteScope::All,
            cx,
        );
    }

    fn raw_member_row(&mut self, chat_id: &str, user_id: &str, cx: &mut Context<Self>) -> Option<Map<String, Value>> {
        let mut member_keys: Vec<String> =
            self.known_member_scopes(chat_id, cx).iter().map(|scope| Self::members_cache_key(scope)).collect();
        member_keys.push(Self::admins_cache_key(chat_id));
        for cache_key in &member_keys {
            let Some(root) = self.member_cache_root(cache_key, cx) else {
                continue;
            };
            if let Some(row) = AnyDictionaryBridge::array_of_dictionaries(root.get(key::MEMBERS))
                .into_iter()
                .find(|row| row_user_id(row) == Some(user_id))
            {
                return Some(row);
            }
        }
        None
    }

    fn make_blocked_row(
        &self,
        member_row: &Map<String, Value>,
        chat_id: &str,
        reason: Option<&str>,
        cx: &mut App,
    ) -> Map<String, Value> {
        let actor = AuthServicesStore::shared(cx).read(cx).profile().cloned();
        let actor_id = actor.as_ref().map(|actor| actor.id.clone()).unwrap_or_default();

        let mut actor_user = Map::new();
        actor_user.insert(key::ID.to_owned(), Value::from(actor_id.clone()));
        actor_user.insert(
            key::NAME.to_owned(),
            Value::from(actor.as_ref().map(|actor| actor.name.clone()).unwrap_or_default()),
        );
        actor_user.insert(
            key::TAG.to_owned(),
            Value::from(actor.as_ref().map(|actor| actor.tag.clone()).unwrap_or_default()),
        );
        actor_user.insert(
            key::AVATAR_URL.to_owned(),
            Value::from(
                actor
                    .as_ref()
                    .map(|actor| actor.avatar_url.clone().unwrap_or_else(|| actor.more.logo.clone()))
                    .unwrap_or_default(),
            ),
        );
        actor_user.insert(
            key::IS_PREMIUM.to_owned(),
            Value::from(actor.as_ref().is_some_and(|actor| actor.more.is_premium)),
        );
        actor_user.insert(
            key::IS_OFFICIAL.to_owned(),
            Value::from(actor.as_ref().is_some_and(|actor| actor.more.is_official)),
        );

        let mut row = Map::new();
        row.insert(
            key::USER_ID.to_owned(),
            member_row.get(key::USER_ID).cloned().unwrap_or_else(|| Value::from("")),
        );
        row.insert(key::CHAT_ID.to_owned(), Value::from(chat_id));
        row.insert(key::BANNED_BY.to_owned(), Value::from(actor_id));
        row.insert(key::BANNED_AT.to_owned(), Value::from((Date::now().time_interval_since_1970() * 1_000.0) as i64));
        row.insert(key::REASON.to_owned(), Value::from(reason.unwrap_or_default()));
        row.insert(key::IS_ACTIVE.to_owned(), Value::Bool(true));
        row.insert(key::BAN_TYPE.to_owned(), Value::from(BanType::Ban.wire()));
        row.insert(key::USER.to_owned(), member_row.get(key::USER).cloned().unwrap_or(Value::Null));
        row.insert(key::BANNED_BY_USER.to_owned(), Value::Object(actor_user));
        row
    }

    fn capture_member_cache_root(
        &self,
        cache_key: &str,
        roots: &mut HashMap<String, Map<String, Value>>,
        cx: &mut App,
    ) {
        if let Some(root) = self.member_cache_root(cache_key, cx) {
            roots.insert(cache_key.to_owned(), root);
        }
    }

    fn member_cache_root(&self, cache_key: &str, cx: &mut App) -> Option<Map<String, Value>> {
        let payload = SaiWsResponseCache::observed_payload(
            cache_key,
            SaiWsCachePreferredLayer::MemoryFirst,
            SaiWsDiskCachePolicy::Persistent,
            cx,
        )?;
        AnyDictionaryBridge::dictionary(Some(&payload))
    }

    fn member_cache_contains(&self, cache_key: &str, user_id: &str, cx: &mut App) -> bool {
        let Some(root) = self.member_cache_root(cache_key, cx) else {
            return false;
        };
        AnyDictionaryBridge::array_of_dictionaries(root.get(key::MEMBERS))
            .iter()
            .any(|row| row_user_id(row) == Some(user_id))
    }

    fn adjust_member_cache_total(&self, cache_key: &str, delta: i64, cx: &mut App) {
        if delta == 0 {
            return;
        }
        let _ = SaiUpdater::update_json_value(
            cache_key,
            |context| {
                let mut root = AnyDictionaryBridge::dictionary(context.payload.as_ref()).unwrap_or_default();
                let mut pagination = AnyDictionaryBridge::dictionary(root.get(key::PAGINATION)).unwrap_or_default();
                if let Some(total) = pagination.get(key::TOTAL_COUNT).and_then(Value::as_i64) {
                    pagination.insert(key::TOTAL_COUNT.to_owned(), Value::from((total + delta).max(0)));
                    root.insert(key::PAGINATION.to_owned(), Value::Object(pagination));
                } else if let Some(total) = root.get(key::TOTAL).and_then(Value::as_i64) {
                    root.insert(key::TOTAL.to_owned(), Value::from((total + delta).max(0)));
                }
                Ok(Value::Object(root))
            },
            SaiUpdaterWriteScope::All,
            cx,
        );
    }

    fn mirror_member_caches_to_rise(&self, chat_id: &str, cx: &mut App) {
        let owned_chat_id = chat_id.to_owned();
        let _ = SaiDebouncer::debounced_action_delay_milliseconds(
            format!("rise-member-mirror-{chat_id}"),
            Box::new(move |cx: &mut App| {
                Self::shared(cx).update(cx, |store, cx| store.persist_member_caches_to_rise(&owned_chat_id, cx));
            }),
            50,
            "riseMemberCacheMirror",
            64,
            None,
            cx,
        );
    }

    fn persist_member_caches_to_rise(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        let Some(domain) = RiseConversationDomainRegistry::shared(cx).read(cx).active() else {
            return;
        };
        let mut resources: Vec<(&'static str, String)> = self
            .known_member_scopes(chat_id, cx)
            .iter()
            .map(|scope| (rise_canon::rise_wire_methods::chat::GET_CHAT_MEMBERS, Self::members_cache_key(scope)))
            .collect();
        resources.push((rise_canon::rise_wire_methods::chat::GET_ADMIN_MEMBERS, Self::admins_cache_key(chat_id)));
        resources.push((rise_canon::rise_wire_methods::chat::GET_BLOCKED_MEMBERS, Self::blocked_cache_key(chat_id)));

        for (method, cache_key) in resources {
            let Some(payload) = SaiWsResponseCache::observed_payload(
                &cache_key,
                SaiWsCachePreferredLayer::MemoryFirst,
                SaiWsDiskCachePolicy::Persistent,
                cx,
            ) else {
                continue;
            };
            let Ok(encoded) = serde_json::to_vec(&payload) else {
                continue;
            };
            let domain = domain.clone();
            drop(RiseAccountRuntime::current().spawn(async move {
                let _ = domain
                    .auxiliary
                    .replace_compatibility_cached_response(
                        rise_canon::rise_wire_methods::chat::SERVICE,
                        method,
                        &cache_key,
                        &encoded,
                    )
                    .await;
            }));
        }
    }

    pub fn apply_member_role_change(
        &mut self,
        chat_id: &str,
        user_id: &str,
        role_ids: Vec<String>,
        display_color: i64,
        cx: &mut Context<Self>,
    ) {
        let mut fields = Map::new();
        fields.insert(key::ROLE_IDS.to_owned(), Value::from(role_ids));
        fields.insert(key::DISPLAY_COLOR.to_owned(), Value::from(display_color));
        self.apply_member_fields_local(chat_id, user_id, fields, cx);
    }

    pub fn moderation_page_limit() -> i64 {
        RiseLimits::client_value("chat_services_store_moderation_page_limit")
    }

    pub const MEMBER_PAGE_LIMIT: i64 = 50;

    pub fn admins_cache_key(chat_id: &str) -> String {
        SaiCache::new(["getAdminMembers", chat_id]).key
    }

    pub fn blocked_cache_key(chat_id: &str) -> String {
        SaiCache::new(["getBlockedMembers", chat_id]).key
    }

    pub fn chat_actions_cache_key(chat_id: &str) -> String {
        SaiCache::new(["getChatActions", chat_id]).key
    }

    fn admins_dto(&self, chat_id: &str, cx: &mut App) -> Option<Rc<GetChatMembersGatewayDTO>> {
        Self::observed_gateway_dto(&Self::admins_cache_key(chat_id), cx)
    }

    pub fn admins(&self, chat_id: &str, cx: &mut App) -> Vec<ChatMember> {
        self.admins_dto(chat_id, cx)
            .map(|dto| dto.members.iter().map(|member| member.to_chat_member()).collect())
            .unwrap_or_default()
    }

    pub fn admins_has_more(&self, chat_id: &str, cx: &mut App) -> bool {
        self.admins_dto(chat_id, cx).and_then(|dto| dto.pagination.as_ref().map(|page| page.has_more)).unwrap_or(false)
    }

    pub fn admins_last_id(&self, chat_id: &str, cx: &mut App) -> Option<String> {
        self.admins_dto(chat_id, cx).and_then(|dto| dto.pagination.as_ref().and_then(|page| page.last_id.clone()))
    }

    fn blocked_dto(&self, chat_id: &str, cx: &mut App) -> Option<Rc<GetBlockedMembersGatewayDTO>> {
        Self::observed_gateway_dto(&Self::blocked_cache_key(chat_id), cx)
    }

    pub fn blocked(&self, chat_id: &str, cx: &mut App) -> Vec<BlockedMember> {
        self.blocked_dto(chat_id, cx)
            .map(|dto| dto.members.iter().map(|member| member.to_blocked_member()).collect())
            .unwrap_or_default()
    }

    pub fn blocked_has_more(&self, chat_id: &str, cx: &mut App) -> bool {
        self.blocked_dto(chat_id, cx).and_then(|dto| dto.pagination.as_ref().map(|page| page.has_more)).unwrap_or(false)
    }

    pub fn blocked_last_id(&self, chat_id: &str, cx: &mut App) -> Option<String> {
        self.blocked_dto(chat_id, cx).and_then(|dto| dto.pagination.as_ref().and_then(|page| page.last_id.clone()))
    }

    pub fn blocked_raw_row(&self, chat_id: &str, user_id: &str, cx: &mut App) -> Option<Map<String, Value>> {
        let dto = self.blocked_dto(chat_id, cx)?;
        let wire = dto.members.iter().find(|member| member.user_id == user_id)?;
        SaiJSONCodec::encode(wire)
    }

    pub fn remove_blocked(&mut self, chat_id: &str, user_id: &str, cx: &mut Context<Self>) {
        let cache_key = Self::blocked_cache_key(chat_id);
        let contained = self.member_cache_contains(&cache_key, user_id, cx);
        let _ = SaiUpdater::remove_row(
            &cache_key,
            key::MEMBERS,
            message_key::USER_ID,
            user_id,
            SaiUpdaterWriteScope::All,
            cx,
        );
        if contained {
            self.adjust_member_cache_total(&cache_key, -1, cx);
        }
        self.mirror_member_caches_to_rise(chat_id, cx);
        cx.notify();
    }

    pub fn restore_blocked(&mut self, row: Map<String, Value>, chat_id: &str, cx: &mut Context<Self>) {
        let cache_key = Self::blocked_cache_key(chat_id);
        let user_id = row_user_id(&row).unwrap_or_default().to_owned();
        let contained = self.member_cache_contains(&cache_key, &user_id, cx);
        let _ = SaiUpdater::insert_row_if_missing(
            &cache_key,
            key::MEMBERS,
            message_key::USER_ID,
            row,
            true,
            SaiUpdaterWriteScope::All,
            cx,
        );
        if !contained {
            self.adjust_member_cache_total(&cache_key, 1, cx);
        }
        self.mirror_member_caches_to_rise(chat_id, cx);
        cx.notify();
    }

    fn chat_actions_dto(&self, chat_id: &str, cx: &mut App) -> Option<Rc<GetChatActionsGatewayDTO>> {
        Self::observed_gateway_dto(&Self::chat_actions_cache_key(chat_id), cx)
    }

    pub fn chat_actions(&self, chat_id: &str, cx: &mut App) -> Vec<ChatAction> {
        self.chat_actions_dto(chat_id, cx)
            .map(|dto| dto.actions.iter().map(|action| action.to_chat_action()).collect())
            .unwrap_or_default()
    }

    pub fn chat_actions_has_more(&self, chat_id: &str, cx: &mut App) -> bool {
        self.chat_actions_dto(chat_id, cx)
            .and_then(|dto| dto.pagination.as_ref().map(|page| page.has_more))
            .unwrap_or(false)
    }

    pub fn chat_actions_last_id(&self, chat_id: &str, cx: &mut App) -> Option<String> {
        self.chat_actions_dto(chat_id, cx).and_then(|dto| dto.pagination.as_ref().and_then(|page| page.last_id.clone()))
    }
}
