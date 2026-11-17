use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use rise_core::RiseIsolated;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;

use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiWsSliceStatus;
use crate::modules::chat::components::chat_search::chat_search_overlay_store::ChatSearchMessageKind;
use crate::modules::chat::engine::core::rise_conversation_auxiliary_repository::{
    RiseConversationAuxiliaryLoadState, RiseConversationAuxiliaryPageSnapshot,
};
use crate::modules::chat::stores::message::message_actions::message_types::{
    GetMessageMessage, MessageMediaItem, MessageMediaKind,
};
use crate::modules::chat::stores::message::message_actions::message_wire_types::MessageWireDTO;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RiseGlobalMessageSearchScope {
    pub kind: ChatSearchMessageKind,
    pub query: String,
}

impl RiseGlobalMessageSearchScope {
    pub fn new(kind: ChatSearchMessageKind, query: &str) -> Option<Self> {
        let normalized = trimming_whitespaces_and_newlines(query).to_lowercase();
        if normalized.is_empty() {
            return None;
        }
        Some(Self { kind, query: normalized })
    }
}

#[derive(Clone, Debug)]
pub struct RiseGlobalMessageSearchResult {
    pub id: String,
    pub position: usize,
    pub message: GetMessageMessage,
    pub media_item: Option<MessageMediaItem>,
    pub link_url: Option<String>,
    pub link_title: Option<String>,
    pub link_description: Option<String>,
    pub link_image_url: Option<String>,
}

#[derive(Clone, Debug)]
pub struct RiseGlobalMessageSearchPresentationSnapshot {
    pub account_id: String,
    pub scope: RiseGlobalMessageSearchScope,
    pub source_revision: i64,
    pub results: Arc<Vec<RiseGlobalMessageSearchResult>>,
    pub source_message_count: usize,
    pub cursor: Option<String>,
    pub has_more: bool,
    pub load_state: RiseConversationAuxiliaryLoadState,
    pub error: Option<String>,
}

#[derive(Clone, Debug)]
pub struct RiseGlobalMessageSearchPresentationState {
    pub results: Arc<Vec<RiseGlobalMessageSearchResult>>,
    pub source_message_count: usize,
    pub cursor: Option<String>,
    pub has_more: bool,
    pub load_state: RiseConversationAuxiliaryLoadState,
    pub error: Option<String>,
    pub revision: u64,
}

impl RiseGlobalMessageSearchPresentationState {
    pub fn empty() -> Self {
        Self {
            results: Arc::new(Vec::new()),
            source_message_count: 0,
            cursor: None,
            has_more: false,
            load_state: RiseConversationAuxiliaryLoadState::Idle,
            error: None,
            revision: 0,
        }
    }

    pub fn status(&self) -> SaiWsSliceStatus {
        match self.load_state {
            RiseConversationAuxiliaryLoadState::Idle => SaiWsSliceStatus::Idle,
            RiseConversationAuxiliaryLoadState::Loading => {
                if self.results.is_empty() {
                    SaiWsSliceStatus::Pending
                } else {
                    SaiWsSliceStatus::Fulfilled
                }
            }
            RiseConversationAuxiliaryLoadState::Loaded => SaiWsSliceStatus::Fulfilled,
            RiseConversationAuxiliaryLoadState::Failed => {
                if self.results.is_empty() {
                    SaiWsSliceStatus::Rejected
                } else {
                    SaiWsSliceStatus::Fulfilled
                }
            }
        }
    }

    pub fn did_load(&self) -> bool {
        self.load_state == RiseConversationAuxiliaryLoadState::Loaded
            || self.load_state == RiseConversationAuxiliaryLoadState::Failed
    }
}

#[derive(Clone)]
struct RiseGlobalMessageSearchPresentationPreparerCacheEntry {
    revision: i64,
    results: Arc<Vec<RiseGlobalMessageSearchResult>>,
    source_message_count: usize,
}

#[derive(Default)]
struct RiseGlobalMessageSearchPresentationPreparerState {
    cache: HashMap<RiseGlobalMessageSearchScope, RiseGlobalMessageSearchPresentationPreparerCacheEntry>,
    order: Vec<RiseGlobalMessageSearchScope>,
}

pub struct RiseGlobalMessageSearchPresentationPreparer {
    state: RiseIsolated<RiseGlobalMessageSearchPresentationPreparerState>,
}

impl Default for RiseGlobalMessageSearchPresentationPreparer {
    fn default() -> Self {
        Self::new()
    }
}

impl RiseGlobalMessageSearchPresentationPreparer {
    const LIMIT: usize = 32;

    pub fn new() -> Self {
        Self { state: RiseIsolated::new(RiseGlobalMessageSearchPresentationPreparerState::default()) }
    }

    pub async fn prepare(
        &self,
        account_id: &str,
        scope: &RiseGlobalMessageSearchScope,
        page: &RiseConversationAuxiliaryPageSnapshot<MessageWireDTO>,
    ) -> RiseGlobalMessageSearchPresentationSnapshot {
        let entry = self.state.with(|state| {
            let entry = match state.cache.get(scope) {
                Some(cached) if cached.revision == page.revision => cached.clone(),
                _ => {
                    let entry = RiseGlobalMessageSearchPresentationPreparerCacheEntry {
                        revision: page.revision,
                        results: Arc::new(Self::make_results(&page.items, scope.kind)),
                        source_message_count: page.items.len(),
                    };
                    state.cache.insert(scope.clone(), entry.clone());
                    entry
                }
            };
            state.order.retain(|other| other != scope);
            state.order.push(scope.clone());
            while state.order.len() > Self::LIMIT {
                let removed = state.order.remove(0);
                state.cache.remove(&removed);
            }
            entry
        });
        RiseGlobalMessageSearchPresentationSnapshot {
            account_id: account_id.to_owned(),
            scope: scope.clone(),
            source_revision: page.revision,
            results: entry.results,
            source_message_count: entry.source_message_count,
            cursor: page.cursor.clone(),
            has_more: page.has_more,
            load_state: page.load_state,
            error: page.error.clone(),
        }
    }

    pub async fn reset(&self) {
        self.state.with(|state| *state = RiseGlobalMessageSearchPresentationPreparerState::default());
    }

    fn make_results(wires: &[MessageWireDTO], kind: ChatSearchMessageKind) -> Vec<RiseGlobalMessageSearchResult> {
        let messages: Vec<GetMessageMessage> = wires.iter().map(MessageWireDTO::to_message).collect();
        match kind {
            ChatSearchMessageKind::Links => Self::link_results(&messages),
            ChatSearchMessageKind::Media => Self::media_results(
                &messages,
                &[MessageMediaKind::Image, MessageMediaKind::Video, MessageMediaKind::Animation, MessageMediaKind::Gif],
            ),
            ChatSearchMessageKind::Files => Self::media_results(&messages, &[MessageMediaKind::Document]),
            ChatSearchMessageKind::Music => Self::media_results(&messages, &[MessageMediaKind::Audio]),
            ChatSearchMessageKind::Voices => {
                Self::media_results(&messages, &[MessageMediaKind::Voice, MessageMediaKind::Audio])
            }
        }
    }

    fn media_results(messages: &[GetMessageMessage], kinds: &[MessageMediaKind]) -> Vec<RiseGlobalMessageSearchResult> {
        let mut results = Vec::with_capacity(messages.len());
        for message in messages {
            for (index, item) in message.media_items.iter().enumerate() {
                if !kinds.contains(&item.kind) {
                    continue;
                }
                let item_id = trimming_whitespaces_and_newlines(&item.id);
                results.push(RiseGlobalMessageSearchResult {
                    id: if item_id.is_empty() {
                        format!("{}-media-{index}", message.id)
                    } else {
                        format!("{}-{item_id}", message.id)
                    },
                    position: results.len(),
                    message: message.clone(),
                    media_item: Some(item.clone()),
                    link_url: None,
                    link_title: None,
                    link_description: None,
                    link_image_url: None,
                });
            }
        }
        results
    }

    fn link_results(messages: &[GetMessageMessage]) -> Vec<RiseGlobalMessageSearchResult> {
        let mut seen: HashSet<String> = HashSet::new();
        let mut results = Vec::with_capacity(messages.len());
        for message in messages {
            let preview = message.link_preview.as_ref();
            let raw = match preview {
                Some(preview) => Some(preview.url.clone()),
                None => Self::first_url(&message.text),
            };
            let Some(url) = raw.as_deref().and_then(Self::normalized) else {
                continue;
            };
            if !seen.insert(url.to_lowercase()) {
                continue;
            }
            results.push(RiseGlobalMessageSearchResult {
                id: format!("{}-link", message.id),
                position: results.len(),
                message: message.clone(),
                media_item: None,
                link_url: Some(url.to_owned()),
                link_title: preview.and_then(|preview| preview.title.clone()),
                link_description: preview.and_then(|preview| preview.description.clone()),
                link_image_url: preview.and_then(|preview| preview.image_url.clone()),
            });
        }
        results
    }

    fn first_url(text: &str) -> Option<String> {
        for word in text.split(char::is_whitespace) {
            if word.starts_with("http://") || word.starts_with("https://") {
                return Some(word.to_owned());
            }
            if word.starts_with("www.") && word.chars().count() > 4 {
                return Some(format!("https://{word}"));
            }
        }
        None
    }

    fn normalized(value: &str) -> Option<&str> {
        let normalized = trimming_whitespaces_and_newlines(value);
        if normalized.is_empty() { None } else { Some(normalized) }
    }
}

#[derive(Default)]
pub struct RiseGlobalMessageSearchPresentationBox {
    pub value: Option<RiseGlobalMessageSearchPresentationState>,
}
