use gpui::{App, Context};
use rise_canon::rise_key::chat as key;
use rise_core::foundation::foundation_date::{Date, RiseClock};
use serde_json::{Map, Value};

use crate::core::lib::any_dictionary_bridge::AnyDictionaryBridge;
use crate::core::lib::sai::core::utilities::sai_cache::SaiCache;
use crate::core::lib::sai::core::utilities::sai_cache_cleaner::SaiCacheCleanLayer;
use crate::core::lib::sai::core::utilities::sai_payload_path::SaiJSONCodec;
use crate::core::lib::sai::modules::sai_updater::sai_updater::{SaiUpdater, SaiUpdaterWriteScope};
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{
    SaiWsCachePreferredLayer, SaiWsDiskCachePolicy,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws_response_cache::SaiWsResponseCache;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::engine::core::rise_conversation_presentation::RiseConversationPresentation;
use crate::modules::chat::engine::messages::rise_message_durable_outbox::RiseSlowModeRefusals;
use crate::modules::chat::stores::chat_guide_sandbox_hooks::chat_guide_sandbox_hooks;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{
    ChatAnalyticsModel, ChatInfo, ChatPerm, ChatPermExt, ChatPermKey, ChatReactionCatalog, ChatReactionConfig,
    ChatSettingsModel, ChatType, LinkedChatInfo,
};
use crate::modules::chat::stores::chats::chat_actions::chat_wire_types::{
    ChatAnalyticsGatewayDTO, ChatAnalyticsGatewayDTOExt, ChatSettingsGatewayDTO, ChatSettingsGatewayDTOExt,
    EffectivePermissionsDTOExt, LinkedChatGatewayDTO, LinkedChatGatewayDTOExt,
};
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::chat::stores::message::message_actions::message_types::{
    GetMessageMessage, MessageDeliveryStatus, MessageThreadOrder,
};
use crate::modules::chat::stores::message::message_services::message_services::MessageServicesStore;

#[derive(Clone)]
pub struct LinkedChatMutationSnapshot {
    settings_root: Option<Map<String, Value>>,
    linked_root: Option<Map<String, Value>>,
}

fn presentation_is_active(cx: &mut App) -> bool {
    RiseConversationPresentation::shared(cx).read(cx).is_active()
}

fn is_own_taken(message: &GetMessageMessage, my_id: &str) -> bool {
    message.sender_id == my_id
        && message.delivery_status != MessageDeliveryStatus::Failed
        && message.delivery_status != MessageDeliveryStatus::Sending
}

impl ChatServicesStore {
    const ATTACHMENT_PICKER_KEYS: [&'static str; 4] =
        [ChatPermKey::PHOTO, ChatPermKey::VIDEO, ChatPermKey::MUSIC, ChatPermKey::FILES];

    pub fn settings_cache_key(chat_id: &str) -> String {
        SaiCache::new(["getChatSettings", chat_id]).key
    }

    pub fn linked_chat_cache_key(chat_id: &str) -> String {
        SaiCache::new(["getLinkedChat", chat_id]).key
    }

    pub fn analytics_cache_key(chat_id: &str) -> String {
        SaiCache::new(["getChatAnalytics", chat_id]).key
    }

    pub fn hydrate_chat_settings_from_disk(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        if presentation_is_active(cx) || self.settings_by_chat.contains_key(chat_id) {
            return;
        }
        let payload = SaiWsResponseCache::observed_payload(
            &Self::settings_cache_key(chat_id),
            SaiWsCachePreferredLayer::DiskFirst,
            SaiWsDiskCachePolicy::Persistent,
            cx,
        );
        let Some(dto) = SaiJSONCodec::decode::<ChatSettingsGatewayDTO>(payload.as_ref()) else {
            return;
        };
        self.settings_by_chat.insert(chat_id.to_owned(), dto.to_model(chat_id));
        cx.notify();
    }

    pub fn hydrate_linked_chat_from_disk(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        if presentation_is_active(cx) || self.linked_chat_by_chat.contains_key(chat_id) {
            return;
        }
        let payload = SaiWsResponseCache::observed_payload(
            &Self::linked_chat_cache_key(chat_id),
            SaiWsCachePreferredLayer::DiskFirst,
            SaiWsDiskCachePolicy::Persistent,
            cx,
        );
        let Some(info) = SaiJSONCodec::decode::<LinkedChatGatewayDTO>(payload.as_ref()).and_then(|dto| dto.to_info())
        else {
            return;
        };
        self.linked_chat_by_chat.insert(chat_id.to_owned(), info);
        cx.notify();
    }

    pub fn hydrate_analytics_from_disk(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        if presentation_is_active(cx) || self.analytics_by_chat.contains_key(chat_id) {
            return;
        }
        let payload = SaiWsResponseCache::observed_payload(
            &Self::analytics_cache_key(chat_id),
            SaiWsCachePreferredLayer::DiskFirst,
            SaiWsDiskCachePolicy::Persistent,
            cx,
        );
        let Some(dto) = SaiJSONCodec::decode::<ChatAnalyticsGatewayDTO>(payload.as_ref()) else {
            return;
        };
        self.analytics_by_chat.insert(chat_id.to_owned(), dto.to_model());
        cx.notify();
    }

    pub fn chat_settings(&self, chat_id: &str, _cx: &App) -> Option<&ChatSettingsModel> {
        self.settings_by_chat.get(chat_id)
    }

    pub fn slow_mode_seconds(&self, chat_id: &str, cx: &mut App) -> i64 {
        let Some(seconds) = self.settings_by_chat.get(chat_id).map(|settings| settings.slow_mode) else {
            return 0;
        };
        if seconds <= 0 {
            return 0;
        }
        let Some(chat) = self.chat_info(chat_id, cx) else {
            return 0;
        };
        if !chat.r#type.is_community()
            || ChatPerm::is_owner_or_legacy_admin_role(chat.role.as_deref())
            || self.moderates_chat(&chat, cx)
        {
            return 0;
        }
        seconds
    }

    pub fn slow_mode_may_apply(&self, chat_id: &str, cx: &mut App) -> bool {
        let Some(chat) = self.chat_info(chat_id, cx) else {
            return false;
        };
        chat.r#type.is_community()
            && !ChatPerm::is_owner_or_legacy_admin_role(chat.role.as_deref())
            && !self.moderates_chat(&chat, cx)
    }

    fn moderates_chat(&self, chat: &ChatInfo, cx: &mut App) -> bool {
        let Some(mask) = self.effective_permissions(&chat.id, None, cx).map(|permissions| permissions.mask()) else {
            return chat.can_manage_chat;
        };
        ChatPerm::has(mask, ChatPerm::MANAGE_MESSAGES) || ChatPerm::has(mask, ChatPerm::MANAGE_GROUP)
    }

    pub fn slow_mode_remaining_seconds(&self, chat_id: &str, _cx: &App) -> i64 {
        self.slow_mode_remaining_seconds_at(chat_id, RiseClock::now())
    }

    pub fn slow_mode_remaining_seconds_at(&self, chat_id: &str, now: Date) -> i64 {
        let Some(until) = self.slow_mode_until.get(chat_id) else {
            return 0;
        };
        (until.time_interval_since(&now).ceil() as i64).max(0)
    }

    pub fn start_slow_mode(&mut self, chat_id: &str, start: Date, cx: &mut Context<Self>) {
        let seconds = self.slow_mode_seconds(chat_id, cx);
        if seconds <= 0 {
            return;
        }

        let now = RiseClock::now();
        let until = start.adding_time_interval(seconds as f64);
        if until <= now {
            return;
        }
        if self.slow_mode_until.get(chat_id).is_some_and(|current| *current >= until) {
            return;
        }

        self.slow_mode_until.retain(|_, value| *value > now);
        self.slow_mode_until.insert(chat_id.to_owned(), until);
        cx.notify();
    }

    pub fn record_own_server_send(&mut self, chat_id: &str, sent_at: f64, cx: &mut Context<Self>) {
        if sent_at <= self.latest_own_server_send_at.get(chat_id).copied().unwrap_or(0.0) {
            return;
        }
        self.latest_own_server_send_at.insert(chat_id.to_owned(), sent_at);
        self.start_slow_mode(chat_id, Date::from_time_interval_since_1970(sent_at), cx);
    }

    pub fn prime_slow_mode_from_history(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        if self.slow_mode_seconds(chat_id, cx) <= 0 {
            return;
        }
        if self.holds_server_slow_mode_wait(chat_id) {
            return;
        }
        let Some(latest_own) = self.latest_own_taken_send(chat_id, cx) else {
            return;
        };
        self.start_slow_mode(chat_id, Date::from_time_interval_since_1970(latest_own), cx);
    }

    fn holds_server_slow_mode_wait(&mut self, chat_id: &str) -> bool {
        let now = RiseClock::now();
        self.slow_mode_server_wait_end.retain(|_, wait_end| *wait_end > now);
        self.slow_mode_server_wait_end.contains_key(chat_id)
    }

    pub fn restart_slow_mode_from_history(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        if let Some(server_wait_end) = RiseSlowModeRefusals::take_wait_end(chat_id)
            && server_wait_end > RiseClock::now()
        {
            self.slow_mode_until.insert(chat_id.to_owned(), server_wait_end);
            self.slow_mode_server_wait_end.insert(chat_id.to_owned(), server_wait_end);
            cx.notify();
            return;
        }
        if self.holds_server_slow_mode_wait(chat_id) {
            return;
        }
        let seconds = self.slow_mode_seconds(chat_id, cx);
        if seconds <= 0 {
            return;
        }
        let Some(latest_own) = self.latest_own_taken_send(chat_id, cx) else {
            return;
        };

        let until = Date::from_time_interval_since_1970(latest_own).adding_time_interval(seconds as f64);
        if until <= RiseClock::now() {
            return;
        }
        self.slow_mode_until.insert(chat_id.to_owned(), until);
        cx.notify();
    }

    fn latest_own_taken_send(&self, chat_id: &str, cx: &mut App) -> Option<f64> {
        let my_id = AuthServicesStore::shared(cx).read(cx).profile().map(|profile| profile.id.clone())?;
        if my_id.is_empty() {
            return None;
        }

        let history = MessageServicesStore::shared(cx).update(cx, |messages, cx| messages.messages(chat_id, cx));
        let latest_in_history = history
            .iter()
            .filter(|message| is_own_taken(message, &my_id))
            .map(|message| message.created_at)
            .fold(None, |latest: Option<f64>, value| Some(latest.map_or(value, |current| current.max(value))));
        let latest_in_row = self
            .chat_info(chat_id, cx)
            .and_then(|chat| chat.last_message)
            .filter(|message| is_own_taken(message, &my_id))
            .map(|message| message.created_at);
        let latest_server_send = self.latest_own_server_send_at.get(chat_id).copied();
        let latest_in_topics = if self.topics_enabled(chat_id, cx) {
            let mut topics = self.topics(chat_id, cx);
            topics.extend(self.general_topic(chat_id, cx));
            topics
                .iter()
                .filter_map(|topic| topic.last_message.as_ref())
                .filter(|message| is_own_taken(message, &my_id))
                .map(|message| message.created_at)
                .fold(None, |latest: Option<f64>, value| Some(latest.map_or(value, |current| current.max(value))))
        } else {
            None
        };

        let latest_own = [latest_in_history, latest_in_row, latest_server_send, latest_in_topics]
            .into_iter()
            .flatten()
            .map(MessageThreadOrder::normalize)
            .fold(None, |latest: Option<f64>, value| Some(latest.map_or(value, |current| current.max(value))))?;
        (latest_own > 0.0).then_some(latest_own)
    }

    pub fn end_slow_mode(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        let Some(until) = self.slow_mode_until.get(chat_id) else {
            return;
        };
        if *until > RiseClock::now().adding_time_interval(1.0) {
            return;
        }
        self.slow_mode_until.remove(chat_id);
        cx.notify();
    }

    pub fn reconcile_slow_mode(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        let seconds = self.slow_mode_seconds(chat_id, cx);
        if seconds <= 0 {
            let exempt_by_role = self.chat_info(chat_id, cx).is_some() && !self.slow_mode_may_apply(chat_id, cx);
            let mut changed = self.slow_mode_until.remove(chat_id).is_some();
            if exempt_by_role {
                changed |= self.slow_mode_server_wait_end.remove(chat_id).is_some();
                let _ = RiseSlowModeRefusals::take_wait_end(chat_id);
            }
            if changed {
                cx.notify();
            }
            return;
        }
        if let Some(until) = self.slow_mode_until.get(chat_id).copied() {
            let cap = RiseClock::now().adding_time_interval(seconds as f64);
            if until > cap {
                self.slow_mode_until.insert(chat_id.to_owned(), cap);
                cx.notify();
            }
        }
        self.prime_slow_mode_from_history(chat_id, cx);
    }

    pub fn member_send_switched_off(&self, chat_id: &str, key: &str, cx: &mut App) -> bool {
        self.member_send_switched_off_mask(
            chat_id,
            key,
            |store, cx| {
                if store.chat_info(chat_id, cx).is_some_and(|chat| chat.topics_enabled) {
                    None
                } else {
                    store.effective_permissions(chat_id, None, cx).map(|permissions| permissions.mask())
                }
            },
            cx,
        )
    }

    pub fn member_send_switched_off_forum_topic_id(
        &self,
        chat_id: &str,
        key: &str,
        forum_topic_id: Option<&str>,
        cx: &mut App,
    ) -> bool {
        if !self.chat_info(chat_id, cx).is_some_and(|chat| chat.topics_enabled) {
            return self.member_send_switched_off(chat_id, key, cx);
        }
        self.member_send_switched_off_mask(
            chat_id,
            key,
            |store, cx| match forum_topic_id.filter(|topic_id| !topic_id.is_empty()) {
                Some(topic_id) => store.memory_only_permissions(chat_id, Some(topic_id), cx),
                None => store.effective_permissions(chat_id, None, cx).map(|permissions| permissions.mask()),
            },
            cx,
        )
    }

    fn member_send_switched_off_mask(
        &self,
        chat_id: &str,
        key: &str,
        mask: impl FnOnce(&Self, &mut App) -> Option<i64>,
        cx: &mut App,
    ) -> bool {
        let Some(chat) = self.chat_info(chat_id, cx) else {
            return false;
        };
        if !chat.r#type.is_community() || ChatPerm::is_owner_or_legacy_admin_role(chat.role.as_deref()) {
            return false;
        }
        if !self.moderates_chat(&chat, cx)
            && let Some(settings) = self.settings_by_chat.get(chat_id)
            && (settings.message_allow.get(key) == Some(&false) || settings.media_allow.get(key) == Some(&false))
        {
            return true;
        }
        let needed = Self::send_permission_bits(key);
        if needed == 0 {
            return false;
        }
        let Some(permissions) = mask(self, cx) else {
            return false;
        };
        if permissions & ChatPerm::VIEW_CHANNELS == 0 {
            return false;
        }
        !ChatPerm::has(permissions, needed)
    }

    pub fn member_attachments_switched_off(
        &self,
        chat_id: &str,
        forum_topic_id: Option<&str>,
        cx: &mut App,
    ) -> bool {
        Self::ATTACHMENT_PICKER_KEYS
            .iter()
            .all(|key| self.member_send_switched_off_forum_topic_id(chat_id, key, forum_topic_id, cx))
    }

    pub fn member_link_previews_switched_off(
        &self,
        chat_id: &str,
        forum_topic_id: Option<&str>,
        cx: &mut App,
    ) -> bool {
        self.member_send_switched_off_forum_topic_id(chat_id, ChatPermKey::WEB_PREVIEWS, forum_topic_id, cx)
    }

    fn send_permission_bits(key: &str) -> i64 {
        match key {
            ChatPermKey::TEXT => ChatPerm::SEND_MESSAGES,
            ChatPermKey::WEB_PREVIEWS => ChatPerm::EMBED_LINKS,
            ChatPermKey::POLLS => ChatPerm::SEND_POLLS,
            ChatPermKey::FILES => ChatPerm::ATTACH_MEDIA | ChatPerm::SEND_DOCUMENTS,
            ChatPermKey::STICKERS | ChatPermKey::GIF => ChatPerm::ATTACH_MEDIA | ChatPerm::SEND_STICKERS,
            ChatPermKey::VOICE | ChatPermKey::VIDEO_MESSAGES => ChatPerm::ATTACH_MEDIA | ChatPerm::SEND_VOICE,
            ChatPermKey::PHOTO | ChatPermKey::VIDEO | ChatPermKey::MUSIC => ChatPerm::ATTACH_MEDIA,
            _ => 0,
        }
    }

    pub fn reaction_config(&self, chat_id: &str, fallback: &[String]) -> ChatReactionConfig {
        if let Some(model) = self.settings_by_chat.get(chat_id) {
            return model.reaction_config();
        }
        if fallback.is_empty() {
            return ChatReactionConfig {
                enabled: true,
                emojis: ChatReactionCatalog::quick_bar(),
                allows_custom_emoji: true,
                restricted_emojis: None,
            };
        }
        let all_emojis = ChatReactionCatalog::all_emojis();
        let is_restricted =
            fallback.len() < all_emojis.len() || !all_emojis.iter().all(|emoji| fallback.contains(emoji));
        ChatReactionConfig {
            enabled: true,
            emojis: fallback.to_vec(),
            allows_custom_emoji: ChatReactionConfig::list_allows_custom_emoji(fallback),
            restricted_emojis: is_restricted.then(|| fallback.to_vec()),
        }
    }

    pub fn linked_chat(&self, chat_id: &str) -> Option<&LinkedChatInfo> {
        self.linked_chat_by_chat.get(chat_id)
    }

    pub fn paid_reactions_allowed(&self, chat_id: &str, r#type: ChatType) -> bool {
        if let Some(allowed) = self.settings_by_chat.get(chat_id).and_then(|settings| settings.paid_reactions_allowed)
        {
            return allowed;
        }
        r#type == ChatType::Channel
    }

    pub fn apply_linked_chat_mutation_local(
        &mut self,
        chat_id: &str,
        linked_chat: Option<LinkedChatInfo>,
        cx: &mut Context<Self>,
    ) -> LinkedChatMutationSnapshot {
        let settings_key = Self::settings_cache_key(chat_id);
        let linked_key = Self::linked_chat_cache_key(chat_id);
        let settings_payload = SaiWsResponseCache::observed_payload(
            &settings_key,
            SaiWsCachePreferredLayer::MemoryFirst,
            SaiWsDiskCachePolicy::Persistent,
            cx,
        );
        let settings_root = AnyDictionaryBridge::dictionary(settings_payload.as_ref());
        let linked_payload = SaiWsResponseCache::observed_payload(
            &linked_key,
            SaiWsCachePreferredLayer::MemoryFirst,
            SaiWsDiskCachePolicy::Persistent,
            cx,
        );
        let linked_root = AnyDictionaryBridge::dictionary(linked_payload.as_ref());

        let mut patched_settings = settings_root.clone().unwrap_or_else(|| {
            let mut root = Map::new();
            root.insert(key::CHAT_ID.to_owned(), Value::from(chat_id));
            root
        });
        patched_settings.insert(
            key::LINKED_CHAT_ID.to_owned(),
            linked_chat.as_ref().map_or(Value::Null, |linked| Value::from(linked.id.as_str())),
        );
        let _ = SaiUpdater::replace_root(&settings_key, patched_settings, SaiUpdaterWriteScope::All, cx);

        if let Some(linked_chat) = linked_chat {
            let mut linked_payload = Map::new();
            linked_payload.insert(key::CHAT_ID.to_owned(), Value::from(linked_chat.id));
            linked_payload.insert(key::TITLE.to_owned(), Value::from(linked_chat.title));
            linked_payload.insert(key::LOGO_URL.to_owned(), Value::from(linked_chat.logo_url));
            linked_payload.insert(key::TYPE.to_owned(), Value::from(linked_chat.r#type));
            let _ = SaiUpdater::replace_root(&linked_key, linked_payload, SaiUpdaterWriteScope::All, cx);
        } else {
            SaiWsResponseCache::remove(&linked_key, SaiCacheCleanLayer::Everywhere, cx);
        }
        cx.notify();
        LinkedChatMutationSnapshot { settings_root, linked_root }
    }

    pub fn restore_linked_chat_mutation(
        &mut self,
        snapshot: &LinkedChatMutationSnapshot,
        chat_id: &str,
        cx: &mut Context<Self>,
    ) {
        let pairs = [
            (Self::settings_cache_key(chat_id), snapshot.settings_root.clone()),
            (Self::linked_chat_cache_key(chat_id), snapshot.linked_root.clone()),
        ];
        for (cache_key, root) in pairs {
            if let Some(root) = root {
                let _ = SaiUpdater::replace_root(&cache_key, root, SaiUpdaterWriteScope::All, cx);
            } else {
                SaiWsResponseCache::remove(&cache_key, SaiCacheCleanLayer::Everywhere, cx);
            }
        }
        cx.notify();
    }

    pub fn resolved_linked_chat(&self, chat_id: &str, cx: &App) -> Option<LinkedChatInfo> {
        if let Some(invented) = (chat_guide_sandbox_hooks().linked_discussion)(chat_id, cx) {
            return invented;
        }
        if let Some(card) = self.linked_chat_by_chat.get(chat_id) {
            return Some(card.clone());
        }

        let linked_id = self.settings_by_chat.get(chat_id)?.linked_chat_id.as_deref()?;
        if linked_id.is_empty() {
            return None;
        }

        if let Some(known) = self.chat_info(linked_id, cx) {
            let r#type = if known.r#type == ChatType::Channel { "CHANNEL" } else { "GROUP" };
            return Some(LinkedChatInfo {
                id: known.id,
                title: known.title,
                logo_url: known.logo_url,
                r#type: r#type.to_owned(),
            });
        }

        Some(LinkedChatInfo { id: linked_id.to_owned(), ..LinkedChatInfo::default() })
    }

    pub fn analytics(&self, chat_id: &str) -> Option<&ChatAnalyticsModel> {
        self.analytics_by_chat.get(chat_id)
    }
}
