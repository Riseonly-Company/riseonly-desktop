use std::rc::Rc;
use std::sync::Arc;

use gpui::{AnyView, App, AppContext};
use rise_theme::ThemeStore;

use crate::modules::chat::components::chat::async_render::async_text_bubble_types::AsyncTextBubbleDeliveryState;
use crate::modules::chat::components::chat::async_render::chat_message_render_snapshot::{
    ChatMessageRenderEntry, ChatMessageRenderEntryGroupFlags, ChatMessageRenderEntryKind,
};
use crate::modules::chat::components::chat::async_render::rise_manual_message_hosting_view::RiseManualMessageHostingView;
use crate::modules::chat::components::chat::async_render::rise_message_list_view::RiseMessageCollectionConfiguration;
use crate::modules::chat::shared::config::message_sizes::MessageSizes;
use crate::modules::chat::stores::message::message_actions::message_types::{GetMessageMessage, MessageDeliveryStatus};

/// base row for the native sticker and document bubbles
/// on desktop the native row is the regular bubble row with the exact height the kind answered
pub enum NativeInteractiveMessageCell {}

impl NativeInteractiveMessageCell {
    /// render entry of the message in the current snapshot
    /// or the same entry built from the message and group when the snapshot doesnt hold it
    pub fn entry(
        message: &GetMessageMessage,
        group: ChatMessageRenderEntryGroupFlags,
        configuration: &RiseMessageCollectionConfiguration,
    ) -> Arc<ChatMessageRenderEntry> {
        if let Some(entry) = configuration.snapshot.entries_by_id.get(&message.id)
            && entry.kind == ChatMessageRenderEntryKind::Message
            && entry.group == Some(group)
            && entry.message.as_ref().is_some_and(|candidate| candidate.id == message.id)
        {
            return entry.clone();
        }
        Arc::new(ChatMessageRenderEntry::new(
            message.id.clone(),
            ChatMessageRenderEntryKind::Message,
            String::new(),
            None,
            None,
            Some(Arc::new(message.clone())),
            Some(group),
            None,
            None,
            None,
            None,
            0,
        ))
    }

    pub fn make_view(
        reusing: Option<AnyView>,
        message: &GetMessageMessage,
        group: ChatMessageRenderEntryGroupFlags,
        configuration: &Rc<RiseMessageCollectionConfiguration>,
        viewport_width: f32,
        height: Option<f32>,
        cx: &mut App,
    ) -> AnyView {
        let entry = Self::entry(message, group, configuration);
        let view = reusing
            .and_then(|view| view.downcast::<RiseManualMessageHostingView>().ok())
            .unwrap_or_else(|| cx.new(RiseManualMessageHostingView::new));
        view.update(cx, |view, cx| {
            view.configure(&entry, configuration, None, Some(viewport_width), false, height, None, cx);
        });
        view.into()
    }
}

/// geometry of the inline and overlay time row the native bubbles price
pub enum NativeMessageMetaView {}

impl NativeMessageMetaView {
    /// text size of the main text meta font, the slider delta is added on top
    pub fn time_font_size() -> f32 {
        (MessageSizes::meta_font_size() + ThemeStore::text_delta_snapshot()).max(1.0)
    }

    pub fn delivery_state(message: &GetMessageMessage) -> AsyncTextBubbleDeliveryState {
        if message.delivery_status == MessageDeliveryStatus::Failed {
            return AsyncTextBubbleDeliveryState::Failed;
        }
        if message.is_pending_outgoing() {
            return AsyncTextBubbleDeliveryState::Sending;
        }
        match message.delivery_status {
            MessageDeliveryStatus::Sending => AsyncTextBubbleDeliveryState::Sending,
            MessageDeliveryStatus::Failed => AsyncTextBubbleDeliveryState::Failed,
            MessageDeliveryStatus::Sent => AsyncTextBubbleDeliveryState::Sent,
            MessageDeliveryStatus::Delivered => AsyncTextBubbleDeliveryState::Delivered,
            MessageDeliveryStatus::Read => AsyncTextBubbleDeliveryState::Read,
        }
    }
}

/// rounds up to the pixel grid like hosted rows
/// native kind prices its row without a window so the grid is the whole point
pub fn native_bubble_pixel_ceil(value: f32) -> f32 {
    (value - 0.0001).ceil()
}
