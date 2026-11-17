use std::borrow::Borrow;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::mem;
use std::rc::Rc;

use gpui::{App, Entity, Subscription};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;

use crate::core::media_viewer::media_viewer_presentation_state::MediaViewerPresentationState;
use crate::core::media_viewer::media_viewer_types::{
    MediaViewerFrameKeys, MediaViewerGalleryItem, MediaViewerGalleryItemKind,
};
use crate::core::ui::clever_image_ui::CleverImageCachePolicy;
use crate::modules::chat::components::chat::media_viewer::chat_media_gallery_builder::ChatMediaGalleryBuilder;
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_source::ChatProfileMediaEntry;
use crate::modules::chat::engine::core::rise_conversation_presentation::RiseConversationPresentation;
use crate::modules::chat::engine::media::rise_chat_media_gallery_repository::{
    RiseChatMediaGalleryIdentity, RiseChatMediaGalleryItem, RiseChatMediaGalleryPayload,
};
use crate::modules::chat::shared::message_permissions::MessagePermissions;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatInfo, ChatType};
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::chat::stores::message::message_actions::message_types::{
    GetMessageMessage, MessageContentType, MessageMediaItem, MessageMediaKind,
};
use crate::modules::chat::stores::message::message_services::message_services::MessageServicesStore;

impl ChatMediaGalleryBuilder {
    /// server id of a message, none while it has none
    /// a message sent from this device keeps its temp id as local id
    /// the feed refuses that as relative id
    pub fn window_message_id(message: &GetMessageMessage) -> Option<String> {
        for raw in [message.server_id.as_deref(), Some(message.id.as_str())] {
            let Some(raw) = raw else {
                continue;
            };
            let id = trimming_whitespaces_and_newlines(raw);
            if Uuid::from_uuid_string(id).is_none() {
                continue;
            }
            return Some(id.to_owned());
        }
        None
    }

    /// feed identity for a locally held attachment, none until the message has a server id
    /// keyed on local slot index and not grid position since the server counts the whole attachment array
    pub fn window_identity_message(message: &GetMessageMessage, item: &MessageMediaItem) -> Option<String> {
        let message_id = Self::window_message_id(message)?;
        Some(RiseChatMediaGalleryIdentity::make(&message_id, item.local_slot_index))
    }

    pub fn window_seed_messages<M: Borrow<GetMessageMessage>>(messages: &[M]) -> Vec<RiseChatMediaGalleryItem> {
        let mut out = Vec::new();
        for message in messages {
            let message: &GetMessageMessage = message.borrow();
            for item in message.grid_media_items() {
                if item.is_cancelled || !matches!(item.kind, MessageMediaKind::Image | MessageMediaKind::Video) {
                    continue;
                }
                let Some(identity) = Self::window_identity_message(message, item) else {
                    continue;
                };
                out.push(RiseChatMediaGalleryItem {
                    id: identity,
                    message_id: message.id.clone(),
                    media_index: item.local_slot_index,
                    created_at: message.created_at,
                    payload: None,
                });
            }
        }
        out
    }

    /// feed identity of a profile grid photo, none for a row whose message has no server id
    pub fn window_identity_profile_entry(entry: &ChatProfileMediaEntry) -> Option<String> {
        let message_id = trimming_whitespaces_and_newlines(&entry.message_id);
        if Uuid::from_uuid_string(message_id).is_none()
            || !matches!(entry.item.kind, MessageMediaKind::Image | MessageMediaKind::Video)
        {
            return None;
        }
        Some(RiseChatMediaGalleryIdentity::make(message_id, entry.item.local_slot_index))
    }

    pub fn window_seed_profile_entries(entries: &[ChatProfileMediaEntry]) -> Vec<RiseChatMediaGalleryItem> {
        let mut message_order: Vec<&str> = Vec::new();
        let mut rows_by_message: HashMap<&str, Vec<RiseChatMediaGalleryItem>> = HashMap::new();
        for entry in entries {
            let Some(identity) = Self::window_identity_profile_entry(entry) else {
                continue;
            };
            if !rows_by_message.contains_key(entry.message_id.as_str()) {
                message_order.push(&entry.message_id);
            }
            rows_by_message.entry(&entry.message_id).or_default().push(RiseChatMediaGalleryItem {
                id: identity,
                message_id: entry.message_id.clone(),
                media_index: entry.item.local_slot_index,
                created_at: entry.created_at,
                payload: None,
            });
        }
        message_order
            .into_iter()
            .rev()
            .flat_map(|message_id| rows_by_message.remove(message_id).unwrap_or_default())
            .collect()
    }

    pub fn viewer_item_id(row: &RiseChatMediaGalleryItem) -> Option<String> {
        MessageMediaItem::stable_render_id_for_slot(&row.message_id, row.media_index)
    }

    pub fn window_topic_id<M: Borrow<GetMessageMessage>>(messages: &[M]) -> Option<String> {
        let mut topic: Option<&str> = None;
        for message in messages {
            let message: &GetMessageMessage = message.borrow();
            let value = trimming_whitespaces_and_newlines(message.topic_id.as_deref().unwrap_or_default());
            if value.is_empty() {
                return None;
            }
            match topic {
                Some(topic) if topic != value => return None,
                Some(_) => {}
                None => topic = Some(value),
            }
        }
        topic.map(str::to_owned)
    }

    pub fn items_window(
        chat: &ChatInfo,
        window: &[RiseChatMediaGalleryItem],
        messages: &[GetMessageMessage],
        cache: &mut HashMap<String, MediaViewerGalleryItem>,
        cx: &mut App,
    ) -> Vec<MediaViewerGalleryItem> {
        let mut messages_by_id: HashMap<&str, &GetMessageMessage> = HashMap::with_capacity(messages.len());
        for message in messages {
            messages_by_id.entry(&message.id).or_insert(message);
        }
        let mut out = Vec::with_capacity(window.len());
        let mut live = HashMap::with_capacity(window.len());
        for row in window {
            if let Some(built) = cache.get(&row.id) {
                live.insert(row.id.clone(), built.clone());
                out.push(built.clone());
                continue;
            }
            let message = messages_by_id.get(row.message_id.as_str()).copied();
            let Some(built) = Self::item(chat, row, message, cx) else {
                continue;
            };
            live.insert(row.id.clone(), built.clone());
            out.push(built);
        }
        *cache = live;
        out
    }

    fn item(
        chat: &ChatInfo,
        row: &RiseChatMediaGalleryItem,
        message: Option<&GetMessageMessage>,
        cx: &mut App,
    ) -> Option<MediaViewerGalleryItem> {
        if let Some(message) = message {
            let media: Vec<&MessageMediaItem> =
                message.grid_media_items().into_iter().filter(|item| !item.is_cancelled).collect();
            if let Some(index) = media.iter().position(|item| item.local_slot_index == row.media_index) {
                return Self::gallery_item(chat, message, media[index], index, media.len(), cx);
            }
        }
        let payload = row.payload.as_ref()?;
        Self::remote_item(chat, row, payload, cx)
    }

    fn remote_item(
        chat: &ChatInfo,
        row: &RiseChatMediaGalleryItem,
        payload: &RiseChatMediaGalleryPayload,
        cx: &mut App,
    ) -> Option<MediaViewerGalleryItem> {
        let content_type = payload.content_type.as_deref().map(MessageContentType::from_raw_value);
        let media = payload.media.to_media_item(row.media_index, &row.message_id, content_type.clone());
        let kind = match media.kind {
            MessageMediaKind::Image => MediaViewerGalleryItemKind::Image,
            MessageMediaKind::Video => MediaViewerGalleryItemKind::Video,
            MessageMediaKind::Animation | MessageMediaKind::Gif => MediaViewerGalleryItemKind::Animation,
            _ => return None,
        };
        let message = Self::remote_message(row, payload, &media, &chat.id, content_type);
        let is_own = Self::is_own(&payload.sender_id, chat, cx);
        let permissions = MessagePermissions::new(&message, chat, is_own);
        let stable_render_id = media.stable_render_id(&row.message_id);
        let frame_registry_id = MediaViewerFrameKeys::chat_bubble(&stable_render_id);
        Some(MediaViewerGalleryItem {
            id: stable_render_id,
            kind,
            url: media.url.clone(),
            thumbnail_url: media.thumbnail_url.clone(),
            width: media.width,
            height: media.height,
            duration: media.duration,
            sender_id: payload.sender_id.clone(),
            sender_name: Self::viewer_sender_name(&payload.sender_name, is_own, chat),
            sender_logo: payload.sender_logo.clone(),
            created_at: row.created_at,
            caption_title: String::new(),
            caption: payload.caption.clone(),
            message_id: row.message_id.clone(),
            index_in_message: usize::try_from(row.media_index).unwrap_or(0),
            media_item_id: media.id.clone(),
            uses_fit_letterbox: payload.total_media_in_message == 1,
            frame_registry_id,
            is_own,
            can_delete: permissions.can_delete_for_me(cx),
            can_save: !permissions.protects_content(),
            can_forward: permissions.can_forward(cx),
            can_reply: permissions.can_reply(cx),
            can_show_in_context: true,
            can_delete_for_me: permissions.can_delete_for_me(cx),
            can_delete_for_everyone: permissions.can_delete_own(cx)
                || permissions.can_delete_private_peer(cx)
                || permissions.can_moderate_others(cx),
            can_set_as_main: false,
            is_album_row: false,
            deletes_from_favourites: chat.r#type == ChatType::Favourites,
            display_source_override: Some(media.display_source()),
            persist_media_to_disk: true,
            post_download: None,
            image_cache_policy: CleverImageCachePolicy::Standard,
        })
    }

    fn remote_message(
        row: &RiseChatMediaGalleryItem,
        payload: &RiseChatMediaGalleryPayload,
        media: &MessageMediaItem,
        chat_id: &str,
        content_type: Option<MessageContentType>,
    ) -> GetMessageMessage {
        let mut message = GetMessageMessage::new(row.message_id.clone());
        message.chat_id = chat_id.to_owned();
        message.sender_id = payload.sender_id.clone();
        message.sender_name = payload.sender_name.clone();
        message.created_at = row.created_at;
        message.text = payload.caption.clone();
        message.media_items = vec![media.clone()];
        message.has_protected_content = payload.has_protected_content;
        message.content_type = content_type.unwrap_or(match media.kind {
            MessageMediaKind::Video => MessageContentType::Video,
            MessageMediaKind::Animation | MessageMediaKind::Gif => MessageContentType::Animation,
            _ => MessageContentType::Image,
        });
        message
    }
}

/// follows landing of rows step by step, lives in its own subscriptions
/// the last step drops them so a viewer that closes or changes gallery ends the follow
struct LandingFollower {
    viewer: Entity<MediaViewerPresentationState>,
    chat: Rc<ChatInfo>,
    topics: Rc<HashSet<Option<String>>>,
    pending: HashSet<String>,
    row_ids: Rc<HashSet<String>>,
    subscriptions: Vec<Subscription>,
}

impl ChatMediaGalleryBuilder {
    pub fn follow_landing_rows<M: Borrow<GetMessageMessage>>(
        viewer: &Entity<MediaViewerPresentationState>,
        chat: &ChatInfo,
        messages: &[M],
        cx: &mut App,
    ) {
        let pending: HashSet<String> = messages
            .iter()
            .map(Borrow::<GetMessageMessage>::borrow)
            .filter(|message| Self::window_message_id(message).is_none())
            .map(|message| message.id.clone())
            .collect();
        if pending.is_empty() {
            return;
        }
        let topics: HashSet<Option<String>> = messages
            .iter()
            .map(Borrow::<GetMessageMessage>::borrow)
            .filter(|message| pending.contains(&message.id))
            .map(|message| message.topic_id.clone())
            .collect();
        Self::start_following_landing(viewer, chat, topics, pending, cx);
    }

    fn start_following_landing(
        viewer: &Entity<MediaViewerPresentationState>,
        chat: &ChatInfo,
        topics: HashSet<Option<String>>,
        pending: HashSet<String>,
        cx: &mut App,
    ) {
        let row_ids: HashSet<String> = viewer.read(cx).items().iter().map(|item| item.id.clone()).collect();
        let follower = Rc::new(RefCell::new(LandingFollower {
            viewer: viewer.clone(),
            chat: Rc::new(chat.clone()),
            topics: Rc::new(topics),
            pending,
            row_ids: Rc::new(row_ids),
            subscriptions: Vec::new(),
        }));
        if !Self::follow_landing_step(&follower, cx) {
            return;
        }
        let on_viewer = follower.clone();
        let viewer_subscription = cx.observe(viewer, move |_, cx| Self::follow_landing_changed(&on_viewer, cx));
        let on_presentation = follower.clone();
        let presentation = RiseConversationPresentation::shared(cx);
        let presentation_subscription =
            cx.observe(&presentation, move |_, cx| Self::follow_landing_changed(&on_presentation, cx));
        let on_services = follower.clone();
        let services = MessageServicesStore::shared(cx);
        let services_subscription =
            cx.observe(&services, move |_, cx| Self::follow_landing_changed(&on_services, cx));
        follower.borrow_mut().subscriptions =
            vec![viewer_subscription, presentation_subscription, services_subscription];
    }

    pub fn live_message(id: &str, chat_id: &str, topic_id: Option<&str>, cx: &mut App) -> Option<GetMessageMessage> {
        let real_chat_id = Self::real_chat_id(chat_id, cx);
        let mut scopes = vec![MessageServicesStore::message_scope(&real_chat_id, topic_id, None)];
        if topic_id.is_none() {
            scopes.push(MessageServicesStore::message_scope(&real_chat_id, Some(""), None));
        }
        for scope in scopes {
            let found = Self::scope_messages(&scope, cx).into_iter().find(|message| message.matches_message_id(id));
            if found.is_some() {
                return found;
            }
        }
        None
    }

    fn real_chat_id(chat_id: &str, cx: &mut App) -> String {
        let store = ChatServicesStore::shared(cx);
        store.read(cx).real_chat_id(chat_id, cx).unwrap_or_else(|| chat_id.to_owned())
    }

    fn scope_messages(scope: &str, cx: &mut App) -> Vec<GetMessageMessage> {
        MessageServicesStore::shared(cx).update(cx, |service, cx| service.messages(scope, cx))
    }

    fn follow_landing_changed(follower: &Rc<RefCell<LandingFollower>>, cx: &mut App) {
        if Self::follow_landing_step(follower, cx) {
            return;
        }
        let subscriptions = mem::take(&mut follower.borrow_mut().subscriptions);
        cx.defer(move |_| drop(subscriptions));
    }

    /// true while some rows are still waiting for their message to land
    fn follow_landing_step(follower: &Rc<RefCell<LandingFollower>>, cx: &mut App) -> bool {
        let (viewer, chat, topics, pending, row_ids) = {
            let follower = RefCell::borrow(follower);
            (
                follower.viewer.clone(),
                follower.chat.clone(),
                follower.topics.clone(),
                follower.pending.clone(),
                follower.row_ids.clone(),
            )
        };
        let state = viewer.read(cx);
        if !state.is_presented() || !state.items().iter().any(|item| row_ids.contains(&item.id)) {
            return false;
        }
        let real_chat_id = Self::real_chat_id(&chat.id, cx);
        let landed = Self::landed_messages(&real_chat_id, &topics, &pending, cx);
        if !landed.is_empty() {
            let current = ChatServicesStore::shared(cx).read(cx).chat_info(&real_chat_id, cx);
            Self::apply_landed_permissions(&landed, &viewer, current.as_ref().unwrap_or(&*chat), cx);
        }
        let mut remaining = pending;
        for message in &landed {
            remaining.remove(&message.id);
        }
        if remaining.is_empty() {
            return false;
        }
        follower.borrow_mut().pending = remaining;
        true
    }

    fn landed_messages(
        chat_id: &str,
        topics: &HashSet<Option<String>>,
        pending: &HashSet<String>,
        cx: &mut App,
    ) -> Vec<GetMessageMessage> {
        let mut out = Vec::new();
        for topic in topics {
            let mut scopes = vec![MessageServicesStore::message_scope(chat_id, topic.as_deref(), None)];
            if topic.is_none() {
                scopes.push(MessageServicesStore::message_scope(chat_id, Some(""), None));
            }
            for scope in scopes {
                out.extend(Self::scope_messages(&scope, cx).into_iter().filter(|message| {
                    pending.contains(&message.id) && Self::window_message_id(message).is_some()
                }));
            }
        }
        out
    }

    fn apply_landed_permissions(
        landed: &[GetMessageMessage],
        viewer: &Entity<MediaViewerPresentationState>,
        chat: &ChatInfo,
        cx: &mut App,
    ) {
        let mut fresh: HashMap<String, MediaViewerGalleryItem> = HashMap::new();
        for message in landed {
            let media: Vec<&MessageMediaItem> =
                message.grid_media_items().into_iter().filter(|item| !item.is_cancelled).collect();
            for (index, item) in media.iter().enumerate() {
                let Some(row) = Self::gallery_item(chat, message, item, index, media.len(), cx) else {
                    continue;
                };
                fresh.insert(row.id.clone(), row);
            }
        }
        let mut rows = viewer.read(cx).items().to_vec();
        let mut changed = false;
        for row in &mut rows {
            let Some(next) = fresh.get(&row.id) else {
                continue;
            };
            row.is_own = next.is_own;
            row.can_delete = next.can_delete;
            row.can_save = next.can_save;
            row.can_forward = next.can_forward;
            row.can_reply = next.can_reply;
            row.can_delete_for_me = next.can_delete_for_me;
            row.can_delete_for_everyone = next.can_delete_for_everyone;
            changed = true;
        }
        if !changed {
            return;
        }
        viewer.update(cx, |viewer, cx| viewer.replace_items(rows, None, cx));
    }
}
