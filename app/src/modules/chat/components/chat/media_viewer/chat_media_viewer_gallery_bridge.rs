use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{App, Context, Entity, Subscription, Task};
use rise_core::foundation::foundation_uuid::Uuid;

use crate::core::desktop_platform::rise_app_events::{RiseAppEvents, RiseSubscription};
use crate::core::media_viewer::media_viewer_presentation_state::{
    MediaViewerGalleryScope, MediaViewerPresentationState,
};
use crate::core::media_viewer::media_viewer_types::{MediaViewerFrameKeys, MediaViewerGalleryItem};
use crate::core::rise_engine_bridge::rise_engine_app_runtime::RISE_ENGINE_CONNECTION_READY;
use crate::modules::chat::components::chat::media_viewer::chat_media_gallery_builder::ChatMediaGalleryBuilder;
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_source::{
    ChatProfileMediaEntry, ChatProfileSource,
};
use crate::modules::chat::engine::media::rise_chat_media_gallery_presentation::RiseChatMediaGalleryPresentation;
use crate::modules::chat::engine::media::rise_chat_media_gallery_repository::RiseChatMediaGalleryWindow;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::chat::stores::message::message_interactions::message_interactions::MessageInteractionsStore;
use crate::modules::chat::stores::message::message_services::message_services::MessageServicesStore;

static PROFILE_GRID_KEY_PREFIX: LazyLock<String> = LazyLock::new(|| MediaViewerFrameKeys::profile_grid(""));

#[derive(Clone, Copy, PartialEq)]
struct WindowStamp {
    session_id: Option<Uuid>,
    revision: i64,
}

/// draws nothing, keeps the media viewer overlay and paged media window in step
/// viewer knows only its rows, window knows nothing of the viewer
/// new session must match the open item first
pub struct ChatMediaViewerGalleryBridge {
    viewer: Entity<MediaViewerPresentationState>,
    gallery_presentation: Entity<RiseChatMediaGalleryPresentation>,
    items_by_identity: HashMap<String, MediaViewerGalleryItem>,
    identity_by_item_id: HashMap<String, String>,
    session_id: Option<Uuid>,
    profile_rows: Option<HashMap<String, MediaViewerGalleryItem>>,
    profile_source_count: usize,
    last_stamp: WindowStamp,
    last_current_index: usize,
    last_is_presented: bool,
    _subscriptions: [Subscription; 2],
    _connection_ready_observer: RiseSubscription,
    _connection_ready_events: Task<()>,
}

impl ChatMediaViewerGalleryBridge {
    pub fn new(viewer: Entity<MediaViewerPresentationState>, cx: &mut Context<Self>) -> Self {
        let gallery_presentation = RiseChatMediaGalleryPresentation::shared(cx);
        let subscriptions = [
            cx.observe(&gallery_presentation, |this, _, cx| this.gallery_window_did_change(cx)),
            cx.observe(&viewer, |this, _, cx| this.viewer_did_change(cx)),
        ];
        let (sender, mut receiver) = mpsc::unbounded::<()>();
        let connection_ready_observer = RiseAppEvents::default_center().observe(RISE_ENGINE_CONNECTION_READY, move |_| {
            let _ = sender.unbounded_send(());
        });
        let connection_ready_events = cx.spawn(async move |this, cx| {
            while receiver.next().await.is_some() {
                if this.update(cx, |this, cx| this.connection_did_become_ready(cx)).is_err() {
                    return;
                }
            }
        });
        let last_stamp = Self::stamp(&gallery_presentation, cx);
        let (last_current_index, last_is_presented) = {
            let state = viewer.read(cx);
            (state.current_index(), state.is_presented())
        };
        Self {
            viewer,
            gallery_presentation,
            items_by_identity: HashMap::new(),
            identity_by_item_id: HashMap::new(),
            session_id: None,
            profile_rows: None,
            profile_source_count: 0,
            last_stamp,
            last_current_index,
            last_is_presented,
            _subscriptions: subscriptions,
            _connection_ready_observer: connection_ready_observer,
            _connection_ready_events: connection_ready_events,
        }
    }

    fn stamp(presentation: &Entity<RiseChatMediaGalleryPresentation>, cx: &App) -> WindowStamp {
        let window = presentation.read(cx).window();
        WindowStamp {
            session_id: window.map(|window| window.session_id),
            revision: window.map_or(-1, |window| window.revision),
        }
    }

    fn media_gallery_window(cx: &mut App) -> Option<RiseChatMediaGalleryWindow> {
        MessageServicesStore::shared(cx).update(cx, |service, cx| service.media_gallery_window(cx))
    }

    fn gallery_window_did_change(&mut self, cx: &mut Context<Self>) {
        let stamp = Self::stamp(&self.gallery_presentation, cx);
        if stamp == self.last_stamp {
            return;
        }
        self.last_stamp = stamp;
        self.apply_window(cx);
    }

    fn viewer_did_change(&mut self, cx: &mut Context<Self>) {
        let (current_index, is_presented, scope) = {
            let state = self.viewer.read(cx);
            (state.current_index(), state.is_presented(), state.gallery_scope())
        };
        if current_index != self.last_current_index {
            self.last_current_index = current_index;
            if let Some(window) = Self::media_gallery_window(cx)
                && Some(window.session_id) == self.session_id
                && scope == MediaViewerGalleryScope::Paged
            {
                self.publish_counter(&window, cx);
            }
            self.report_focus(cx);
        }
        if is_presented != self.last_is_presented {
            self.last_is_presented = is_presented;
            if is_presented {
                self.apply_window(cx);
            } else {
                self.end_session(cx);
            }
        }
    }

    fn connection_did_become_ready(&mut self, cx: &mut Context<Self>) {
        let Some(session_id) = self.session_id else {
            return;
        };
        let identity = {
            let state = self.viewer.read(cx);
            if !state.is_presented() {
                return;
            }
            state
                .items()
                .get(state.current_index())
                .and_then(|item| self.identity_by_item_id.get(&item.id))
                .cloned()
        };
        MessageInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.media_gallery_did_reconnect(session_id, identity.as_deref(), cx);
        });
    }

    fn apply_window(&mut self, cx: &mut Context<Self>) {
        let Some(window) = Self::media_gallery_window(cx) else {
            return;
        };
        let viewer = self.viewer.clone();
        let (open_item_id, open_frame_key, current_index) = {
            let state = viewer.read(cx);
            if !state.is_presented() || state.gallery_scope() != MediaViewerGalleryScope::Paged {
                return;
            }
            let Some(open) = state.items().get(state.current_index()) else {
                return;
            };
            (open.id.clone(), open.frame_registry_id.clone(), state.current_index())
        };
        let owns_open_item =
            self.session_id == Some(window.session_id) && self.identity_by_item_id.contains_key(&open_item_id);
        let mut expected: Vec<String> = Vec::with_capacity(window.items.len());
        let mut identities: HashMap<String, String> = HashMap::with_capacity(window.items.len());
        let mut profile_frame_keys: HashMap<String, String> = HashMap::new();
        let mut seeded_item_ids: Vec<String> = Vec::new();
        for row in &window.items {
            let Some(item_id) = ChatMediaGalleryBuilder::viewer_item_id(row) else {
                continue;
            };
            expected.push(item_id.clone());
            identities.insert(item_id.clone(), row.id.clone());
            if row.payload.is_none() {
                seeded_item_ids.push(item_id.clone());
            }
            profile_frame_keys.insert(
                item_id,
                MediaViewerFrameKeys::profile_grid(&ChatProfileMediaEntry::entry_id(&row.message_id, row.media_index)),
            );
        }
        if !identities.contains_key(&open_item_id) && !owns_open_item {
            return;
        }
        if self.session_id != Some(window.session_id) {
            self.session_id = Some(window.session_id);
            self.profile_source_count = 0;
            self.items_by_identity.clear();
            self.profile_rows = open_frame_key
                .starts_with(PROFILE_GRID_KEY_PREFIX.as_str())
                .then(|| Self::rows_by_id(viewer.read(cx).items()));
        }
        self.identity_by_item_id = identities.clone();
        if window.items.is_empty() {
            viewer.update(cx, |viewer, cx| viewer.complete_flyback_handoff(cx));
            return;
        }
        if expected.iter().eq(viewer.read(cx).items().iter().map(|item| &item.id)) {
            self.publish_counter(&window, cx);
            self.report_focus(cx);
            return;
        }
        let Some(chat) = ChatServicesStore::shared(cx).read(cx).chat_info(&window.chat_id, cx) else {
            return;
        };
        let scope = MessageServicesStore::message_scope(&window.chat_id, window.topic_id.as_deref(), None);
        let mut messages = MessageServicesStore::shared(cx).update(cx, |service, cx| service.messages(&scope, cx));
        if window.topic_id.is_none() {
            let general = MessageServicesStore::message_scope(&window.chat_id, Some(""), None);
            messages.extend(MessageServicesStore::shared(cx).update(cx, |service, cx| service.messages(&general, cx)));
        }
        let viewer_items = viewer.read(cx).items().to_vec();
        let mut cache = self.items_by_identity.clone();
        if self.profile_rows.is_none() && !seeded_item_ids.is_empty() {
            let on_screen = Self::rows_by_id(&viewer_items);
            for item_id in &seeded_item_ids {
                let Some(identity) = identities.get(item_id) else {
                    continue;
                };
                if cache.contains_key(identity) {
                    continue;
                }
                if let Some(row) = on_screen.get(item_id) {
                    cache.insert(identity.clone(), row.clone());
                }
            }
        }
        if let Some(mut known) = self.profile_rows.take() {
            let is_missing_seed = seeded_item_ids.iter().any(|item_id| {
                !known.contains_key(item_id)
                    && identities.get(item_id).is_some_and(|identity| !cache.contains_key(identity))
            });
            if is_missing_seed {
                let entries = ChatProfileSource::media_items(&window.chat_id, window.topic_id.as_deref(), cx);
                if entries.len() != self.profile_source_count {
                    self.profile_source_count = entries.len();
                    for row in ChatMediaGalleryBuilder::items_profile_entries(&chat, &entries, cx) {
                        known.entry(row.id.clone()).or_insert(row);
                    }
                }
            }
            for (item_id, identity) in &identities {
                if cache.contains_key(identity) {
                    continue;
                }
                if let Some(row) = known.get(item_id) {
                    cache.insert(identity.clone(), row.clone());
                }
            }
            self.profile_rows = Some(known);
        }
        let mut rows = ChatMediaGalleryBuilder::items_window(&chat, &window.items, &messages, &mut cache, cx);
        self.items_by_identity = cache;
        if self.profile_rows.is_some() {
            for row in &mut rows {
                if row.frame_registry_id.starts_with(PROFILE_GRID_KEY_PREFIX.as_str()) {
                    continue;
                }
                if let Some(key) = profile_frame_keys.get(&row.id) {
                    row.frame_registry_id = key.clone();
                }
            }
        }
        if rows.is_empty() {
            return;
        }
        let selected_item_id = {
            let surviving_ids: HashSet<&str> = rows.iter().map(|row| row.id.as_str()).collect();
            if surviving_ids.contains(open_item_id.as_str()) {
                open_item_id
            } else {
                viewer_items
                    .iter()
                    .skip(current_index + 1)
                    .find(|item| surviving_ids.contains(item.id.as_str()))
                    .or_else(|| {
                        viewer_items
                            .iter()
                            .take(current_index)
                            .rev()
                            .find(|item| surviving_ids.contains(item.id.as_str()))
                    })
                    .map_or_else(|| rows[current_index.min(rows.len() - 1)].id.clone(), |item| item.id.clone())
            }
        };
        viewer.update(cx, |viewer, cx| viewer.replace_items(rows, Some(selected_item_id.as_str()), cx));
        self.publish_counter(&window, cx);
        self.report_focus(cx);
    }

    fn rows_by_id(items: &[MediaViewerGalleryItem]) -> HashMap<String, MediaViewerGalleryItem> {
        let mut rows = HashMap::with_capacity(items.len());
        for item in items {
            rows.entry(item.id.clone()).or_insert_with(|| item.clone());
        }
        rows
    }

    /// looked up by row identity, never by arithmetic on the viewer index
    /// undrawable rows are dropped on the way so an offset would be off by one
    /// unknown or invalid pair hides the counter
    fn publish_counter(&self, window: &RiseChatMediaGalleryWindow, cx: &mut Context<Self>) {
        let position = {
            let state = self.viewer.read(cx);
            state
                .items()
                .get(state.current_index())
                .and_then(|item| self.identity_by_item_id.get(&item.id))
                .and_then(|identity| window.positions.get(identity))
                .copied()
        };
        let (position, total) = match (position, window.total) {
            (Some(position), Some(total)) => (Some(position), Some(total)),
            _ => (None, None),
        };
        self.viewer.update(cx, |viewer, cx| viewer.set_gallery_counter(position, total, cx));
    }

    fn report_focus(&self, cx: &mut Context<Self>) {
        let Some(window) = Self::media_gallery_window(cx) else {
            return;
        };
        if Some(window.session_id) != self.session_id {
            return;
        }
        let identity = {
            let state = self.viewer.read(cx);
            if !state.is_presented() {
                return;
            }
            state
                .items()
                .get(state.current_index())
                .and_then(|item| self.identity_by_item_id.get(&item.id))
                .cloned()
        };
        let Some(identity) = identity else {
            return;
        };
        MessageInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.media_gallery_did_focus(window.session_id, &identity, cx);
        });
    }

    fn end_session(&mut self, cx: &mut Context<Self>) {
        let Some(session_id) = self.session_id else {
            return;
        };
        MessageInteractionsStore::shared(cx).update(cx, |store, cx| store.close_media_gallery(session_id, cx));
        self.session_id = None;
        self.profile_rows = None;
        self.profile_source_count = 0;
        self.items_by_identity = HashMap::new();
        self.identity_by_item_id = HashMap::new();
    }
}
