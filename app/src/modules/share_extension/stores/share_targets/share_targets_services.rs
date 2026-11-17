use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

use gpui::{App, Context, Entity};
use rise_canon::RiseLimits;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use serde::Deserialize;
use serde::de::IntoDeserializer;
use serde::de::value::{Error as ValueError, StrDeserializer};

use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::canon_identity::RiseIdentity;
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::core::utilities::sai_cache::SaiCache;
use crate::core::lib::sai::core::utilities::sai_payload_path::SaiJSONCodec;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiSliceState;
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{
    SaiWsCachePreferredLayer, SaiWsDiskCachePolicy,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws_response_cache::SaiWsResponseCache;
use crate::core::lib::sai::sai::Sai;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::engine::core::rise_conversation_presentation::RiseConversationPresentation;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::share_extension::services::rise_share_app_bridge::RiseShareAppBridge;
use crate::modules::share_extension::shared::rise_share_models::{
    RiseShareSection, RiseShareTarget, RiseShareTargetKind, RiseShareTopic, RiseShareTopicStructure,
};
use crate::modules::share_extension::stores::share_targets::share_targets_actions::ShareTargetsActionsStore;
use crate::modules::share_extension::stores::share_targets::share_targets_wire_types::{
    GetShareTargetsGatewayDTO, GetShareTopicsGatewayDTO, ShareTargetWireDTO,
};

pub struct ShareTargetsServicesStore {
    targets: Vec<RiseShareTarget>,
    last_error: Option<String>,
    picker_pages: HashMap<String, PickerPages>,
}

#[derive(Clone)]
struct PickerPages {
    targets: Vec<RiseShareTarget>,
    next_offset: i64,
    has_more: bool,
}

impl PickerPages {
    fn new(next_offset: i64, has_more: bool) -> Self {
        Self {
            targets: Vec::new(),
            next_offset,
            has_more,
        }
    }
}

impl ShareTargetsServicesStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            targets: Vec::new(),
            last_error: None,
            picker_pages: HashMap::new(),
        }
    }

    pub fn targets(&self) -> &[RiseShareTarget] {
        &self.targets
    }

    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    pub fn targets_cache_key(
        user_id: &str,
        query: Option<&str>,
        has_media: bool,
        has_link: bool,
        has_documents: bool,
    ) -> String {
        let normalized_query = query
            .map(|query| trimming_whitespaces_and_newlines(query).to_lowercase())
            .unwrap_or_default();
        SaiCache::new([
            "getShareTargets",
            user_id,
            normalized_query.as_str(),
            if has_media { "media" } else { "no-media" },
            if has_link { "link" } else { "no-link" },
            if has_documents { "documents" } else { "no-documents" },
        ])
        .key
    }

    pub fn topics_cache_key(
        user_id: &str,
        chat_id: &str,
        has_media: bool,
        has_link: bool,
        has_documents: bool,
    ) -> String {
        SaiCache::new([
            "getShareTopics",
            user_id,
            chat_id,
            if has_media { "media" } else { "no-media" },
            if has_link { "link" } else { "no-link" },
            if has_documents { "documents" } else { "no-documents" },
        ])
        .key
    }

    pub fn apply(&mut self, dto: &GetShareTargetsGatewayDTO, cx: &mut Context<Self>) {
        let mut previous_writable_ids: HashSet<String> = self
            .targets
            .iter()
            .filter(|target| target.can_send)
            .map(|target| target.id.clone())
            .collect();
        previous_writable_ids.extend(RiseShareAppBridge::persisted_writable_target_ids_for_current_account(cx));
        let removed = ChatServicesStore::shared(cx).read(cx).locally_removed_chat_ids();
        let mut targets: Vec<RiseShareTarget> = dto
            .targets
            .iter()
            .filter_map(Self::map_target)
            .filter(|target| target.can_send && !removed.contains(&target.id))
            .collect();
        targets.sort_by(Self::activity_order);
        targets.truncate(usize::try_from(RiseLimits::service_value("get_share_targets_limit_clamp")).unwrap_or(0));
        self.targets = targets;
        let fresh_writable_ids: HashSet<String> = self
            .targets
            .iter()
            .filter(|target| target.can_send)
            .map(|target| target.id.clone())
            .collect();
        RiseShareAppBridge::remove_donated_interactions(&RiseShareAppBridge::removed_writable_target_ids(
            &previous_writable_ids,
            &fresh_writable_ids,
        ));
        self.last_error = None;
        cx.notify();
        RiseShareAppBridge::write_snapshot(&self.targets, cx);
    }

    pub fn apply_error(&mut self, error: &anyhow::Error, cx: &mut Context<Self>) {
        self.last_error = Some(error.to_string());
        cx.notify();
        let targets = self.targets.clone();
        cx.defer(move |cx| RiseShareAppBridge::write_snapshot_keeping_persisted_targets(&targets, cx));
        Sai::log(
            "ShareExtension",
            || format!("get_share_targets failed: {error}"),
            SaiLogKind::Network,
            cx,
        );
    }

    pub fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        self.targets.clear();
        self.last_error = None;
        self.picker_pages.clear();
        cx.notify();
    }

    pub fn reset_picker_pages(&mut self, cx: &mut Context<Self>) {
        self.picker_pages.clear();
        cx.notify();
    }

    pub fn next_picker_page_offset(
        &self,
        query: Option<&str>,
        has_media: bool,
        has_link: bool,
        has_documents: bool,
        cx: &mut App,
    ) -> Option<i64> {
        let key = Self::picker_base_key(query, has_media, has_link, has_documents, cx)?;
        if let Some(pages) = self.picker_pages.get(&key) {
            return pages.has_more.then_some(pages.next_offset);
        }
        let first = Self::cached_targets_dto(&key, cx)?;
        first.has_more.then_some(first.next_offset)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn append_picker_page(
        &mut self,
        dto: &GetShareTargetsGatewayDTO,
        requested_offset: i64,
        query: Option<&str>,
        has_media: bool,
        has_link: bool,
        has_documents: bool,
        cx: &mut Context<Self>,
    ) -> usize {
        let Some(key) = Self::picker_base_key(query, has_media, has_link, has_documents, cx) else {
            return 0;
        };
        if self.next_picker_page_offset(query, has_media, has_link, has_documents, cx) != Some(requested_offset) {
            return 0;
        }
        let mut pages = self
            .picker_pages
            .get(&key)
            .cloned()
            .unwrap_or_else(|| PickerPages::new(requested_offset, true));
        let mut known: HashSet<String> = pages.targets.iter().map(|target| target.id.clone()).collect();
        if let Some(cached) = Self::cached_targets_dto(&key, cx) {
            known.extend(cached.targets.iter().filter_map(Self::map_target).map(|target| target.id));
        }
        let fresh: Vec<RiseShareTarget> = dto
            .targets
            .iter()
            .filter_map(Self::map_target)
            .filter(|target| known.insert(target.id.clone()))
            .collect();
        let fresh_count = fresh.len();
        pages.targets.extend(fresh);
        let next = dto.next_offset;
        pages.has_more = dto.has_more && next > requested_offset;
        pages.next_offset = next;
        self.picker_pages.insert(key, pages);
        cx.notify();
        fresh_count
    }

    pub fn end_picker_pages(
        &mut self,
        query: Option<&str>,
        has_media: bool,
        has_link: bool,
        has_documents: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(key) = Self::picker_base_key(query, has_media, has_link, has_documents, cx) else {
            return;
        };
        let mut pages = self
            .picker_pages
            .get(&key)
            .cloned()
            .unwrap_or_else(|| PickerPages::new(0, false));
        pages.has_more = false;
        self.picker_pages.insert(key, pages);
        cx.notify();
    }

    pub fn is_picker_search_scanning(
        &self,
        query: Option<&str>,
        has_media: bool,
        has_link: bool,
        has_documents: bool,
        cx: &mut App,
    ) -> bool {
        let normalized = query.map(trimming_whitespaces_and_newlines).unwrap_or_default();
        if normalized.is_empty()
            || !self
                .picker_targets(Some(normalized), has_media, has_link, has_documents, cx)
                .is_empty()
        {
            return false;
        }
        self.next_picker_page_offset(Some(normalized), has_media, has_link, has_documents, cx)
            .is_some()
    }

    fn picker_base_key(
        query: Option<&str>,
        has_media: bool,
        has_link: bool,
        has_documents: bool,
        cx: &mut App,
    ) -> Option<String> {
        let user_id = Self::wire_user_id(cx)?;
        let normalized = query
            .map(trimming_whitespaces_and_newlines)
            .filter(|normalized| !normalized.is_empty());
        Some(Self::targets_cache_key(&user_id, normalized, has_media, has_link, has_documents))
    }

    fn cached_targets_dto(key: &str, cx: &mut App) -> Option<GetShareTargetsGatewayDTO> {
        let payload = Self::observed_cache_payload(key, cx)?;
        SaiJSONCodec::decode::<GetShareTargetsGatewayDTO>(Some(&payload))
    }

    pub fn picker_targets(
        &self,
        query: Option<&str>,
        has_media: bool,
        has_link: bool,
        has_documents: bool,
        cx: &mut App,
    ) -> Vec<RiseShareTarget> {
        let Some(user_id) = Self::wire_user_id(cx) else {
            return Vec::new();
        };

        let key = Self::targets_cache_key(&user_id, query, has_media, has_link, has_documents);
        let Some(dto) = Self::cached_targets_dto(&key, cx) else {
            return Vec::new();
        };
        let removed = ChatServicesStore::shared(cx).read(cx).locally_removed_chat_ids();
        let first = Self::sorted_targets(
            dto.targets
                .iter()
                .filter_map(Self::map_target)
                .filter(|target| !removed.contains(&target.id))
                .collect(),
        );
        let Some(pages) = self.picker_pages.get(&key).filter(|pages| !pages.targets.is_empty()) else {
            return first;
        };
        let mut seen: HashSet<String> = first.iter().map(|target| target.id.clone()).collect();
        let older = Self::sorted_targets(
            pages
                .targets
                .iter()
                .filter(|target| !removed.contains(&target.id) && seen.insert(target.id.clone()))
                .cloned()
                .collect(),
        );
        first.into_iter().chain(older).collect()
    }

    pub fn picker_topic_structure(
        &self,
        chat_id: &str,
        has_media: bool,
        has_link: bool,
        has_documents: bool,
        cx: &mut App,
    ) -> RiseShareTopicStructure<RiseShareTopic> {
        let empty = || RiseShareTopicStructure {
            sections: Vec::new(),
            topics: Vec::new(),
        };
        let Some(user_id) = Self::wire_user_id(cx) else {
            return empty();
        };
        if chat_id.is_empty() {
            return empty();
        }

        let key = Self::topics_cache_key(&user_id, chat_id, has_media, has_link, has_documents);
        let Some(payload) = Self::observed_cache_payload(&key, cx) else {
            return empty();
        };

        let dto = SaiJSONCodec::decode::<GetShareTopicsGatewayDTO>(Some(&payload));
        let sections: Vec<RiseShareSection> = dto
            .as_ref()
            .map(|dto| dto.sections.as_slice())
            .unwrap_or_default()
            .iter()
            .filter_map(|section| {
                let section_id = trimming_whitespaces_and_newlines(&section.section_id);
                let section_chat_id = trimming_whitespaces_and_newlines(&section.chat_id);
                let belongs_to_chat = section_chat_id.is_empty() || section_chat_id == chat_id;
                if section_id.is_empty() || !belongs_to_chat {
                    return None;
                }
                let created_at = RiseIdentity::milliseconds(section.created_at);
                Some(RiseShareSection {
                    id: section_id.to_owned(),
                    chat_id: if section_chat_id.is_empty() {
                        chat_id.to_owned()
                    } else {
                        section_chat_id.to_owned()
                    },
                    name: trimming_whitespaces_and_newlines(&section.name).to_owned(),
                    position: section.position,
                    created_at_ms: created_at,
                })
            })
            .collect();
        let topics: Vec<RiseShareTopic> = dto
            .as_ref()
            .map(|dto| dto.topics.as_slice())
            .unwrap_or_default()
            .iter()
            .filter_map(|topic| {
                let topic_chat_id = trimming_whitespaces_and_newlines(&topic.chat_id);
                let topic_id = trimming_whitespaces_and_newlines(&topic.topic_id);
                let addressable = topic.is_general || !topic_id.is_empty();
                if !topic.can_send || topic_chat_id != chat_id || !addressable {
                    return None;
                }
                Some(RiseShareTopic {
                    id: topic_id.to_owned(),
                    chat_id: topic_chat_id.to_owned(),
                    title: trimming_whitespaces_and_newlines(&topic.title).to_owned(),
                    icon: trimming_whitespaces_and_newlines(&topic.icon).to_owned(),
                    section_id: trimming_whitespaces_and_newlines(&topic.section_id).to_owned(),
                    position: topic.position,
                    is_general: topic.is_general,
                    can_send: true,
                })
            })
            .collect();
        RiseShareTopicStructure { sections, topics }
    }

    pub fn picker_topics(
        &self,
        chat_id: &str,
        has_media: bool,
        has_link: bool,
        has_documents: bool,
        cx: &mut App,
    ) -> Vec<RiseShareTopic> {
        let mut topics = self
            .picker_topic_structure(chat_id, has_media, has_link, has_documents, cx)
            .topics;
        topics.sort_by(|lhs, rhs| {
            rhs.is_general
                .cmp(&lhs.is_general)
                .then_with(|| lhs.position.cmp(&rhs.position))
                .then_with(|| lhs.id.cmp(&rhs.id))
        });
        topics
    }

    pub fn picker_targets_request_state(
        &self,
        query: Option<&str>,
        has_media: bool,
        has_link: bool,
        has_documents: bool,
        cx: &mut App,
    ) -> (bool, Option<String>) {
        let normalized_query = query.map(trimming_whitespaces_and_newlines).unwrap_or_default();
        let uses_snapshot_slice = normalized_query.is_empty() && !has_media && !has_link && !has_documents;
        if uses_snapshot_slice {
            let actions = ShareTargetsActionsStore::shared(cx);
            let slice = &actions.read(cx).get_share_targets_sai;
            return (slice.is_pending(), slice.error());
        }

        let Some(user_id) = Self::wire_user_id(cx) else {
            return (false, None);
        };

        let key = Self::targets_cache_key(
            &user_id,
            (!normalized_query.is_empty()).then_some(normalized_query),
            has_media,
            has_link,
            has_documents,
        );
        let actions = ShareTargetsActionsStore::shared(cx);
        let actions = actions.read(cx);
        let slice = if normalized_query.is_empty() {
            actions.get_share_target_base_slices.get(&key)
        } else {
            actions.get_share_target_search_slices.get(&key)
        };
        (
            slice.is_some_and(|slice| slice.is_pending()),
            slice.and_then(|slice| slice.error()),
        )
    }

    pub fn picker_topics_request_state(
        &self,
        chat_id: &str,
        has_media: bool,
        has_link: bool,
        has_documents: bool,
        cx: &mut App,
    ) -> (bool, Option<String>) {
        let Some(user_id) = Self::wire_user_id(cx) else {
            return (false, None);
        };
        if chat_id.is_empty() {
            return (false, None);
        }

        let key = Self::topics_cache_key(&user_id, chat_id, has_media, has_link, has_documents);
        let actions = ShareTargetsActionsStore::shared(cx);
        let slice = actions.read(cx).get_share_topic_slices.get(&key);
        (
            slice.is_some_and(|slice| slice.is_pending()),
            slice.and_then(|slice| slice.error()),
        )
    }

    pub fn map_target(dto: &ShareTargetWireDTO) -> Option<RiseShareTarget> {
        let id = trimming_whitespaces_and_newlines(&dto.chat_id);
        if id.is_empty() || !dto.can_send {
            return None;
        }
        let kind = Self::target_kind(&dto.r#type.to_lowercase()).unwrap_or(RiseShareTargetKind::PrivateChat);
        Some(RiseShareTarget::new(
            id.to_owned(),
            trimming_whitespaces_and_newlines(&dto.title).to_owned(),
            Some(dto.avatar_url.clone()),
            Some(dto.color_key.clone()),
            kind,
            dto.topics_enabled,
            true,
            RiseIdentity::milliseconds(dto.last_activity),
        ))
    }

    fn sorted_targets(values: Vec<RiseShareTarget>) -> Vec<RiseShareTarget> {
        let mut sorted: Vec<RiseShareTarget> = values.into_iter().filter(|target| target.can_send).collect();
        sorted.sort_by(Self::activity_order);
        sorted
    }

    fn activity_order(lhs: &RiseShareTarget, rhs: &RiseShareTarget) -> Ordering {
        rhs.last_activity_ms
            .cmp(&lhs.last_activity_ms)
            .then_with(|| lhs.id.cmp(&rhs.id))
    }

    fn target_kind(raw: &str) -> Option<RiseShareTargetKind> {
        let deserializer: StrDeserializer<'_, ValueError> = raw.into_deserializer();
        RiseShareTargetKind::deserialize(deserializer).ok()
    }

    fn observed_cache_payload(key: &str, cx: &mut App) -> Option<serde_json::Value> {
        if RiseConversationPresentation::shared(cx).read(cx).is_active() {
            SaiWsResponseCache::observed_memory_only(key, cx)
        } else {
            SaiWsResponseCache::observed_payload(
                key,
                SaiWsCachePreferredLayer::MemoryFirst,
                SaiWsDiskCachePolicy::PreserveExisting,
                cx,
            )
        }
    }

    fn wire_user_id(cx: &mut App) -> Option<String> {
        AuthServicesStore::shared(cx)
            .read(cx)
            .effective_user_id_for_wire()
            .filter(|user_id| !user_id.is_empty())
    }
}
