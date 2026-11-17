use std::rc::Rc;

use gpui::{AnyView, App, Window};
use rise_theme::AppTheme;

use crate::modules::chat::components::chat::async_render::chat_message_render_snapshot::ChatMessageRenderEntryGroupFlags;
use crate::modules::chat::components::chat::async_render::rise_message_list_view::RiseMessageCollectionConfiguration;
use crate::modules::chat::components::chat::native_bubbles::media::rise_native_media_layout::RiseNativeMediaLayout;
use crate::modules::chat::components::chat::native_bubbles::media::rise_native_media_message_cell::RiseNativeMediaMessageCell;
use crate::modules::chat::components::chat::native_bubbles::native_bubble_registry::NativeBubbleKind;
use crate::modules::chat::stores::message::message_actions::message_types::GetMessageMessage;

/// photo, video or album rows with or without a plain caption
/// the cell is in the native media message cell and geometry in the media layout file
pub enum NativeCaptionMediaBubble {}

impl NativeBubbleKind for NativeCaptionMediaBubble {
    const REUSE_IDENTIFIER: &'static str = RiseNativeMediaMessageCell::REUSE_IDENTIFIER;

    fn row_height(
        message: &GetMessageMessage,
        group: ChatMessageRenderEntryGroupFlags,
        configuration: &RiseMessageCollectionConfiguration,
        viewport_width: f32,
        cx: &mut App,
    ) -> Option<f32> {
        RiseNativeMediaLayout::make(message, group, configuration, viewport_width, cx).map(|layout| layout.row_height)
    }

    fn make_view(
        reusing: Option<AnyView>,
        message: &GetMessageMessage,
        group: ChatMessageRenderEntryGroupFlags,
        configuration: &Rc<RiseMessageCollectionConfiguration>,
        viewport_width: f32,
        theme: &AppTheme,
        _window: &mut Window,
        cx: &mut App,
    ) -> AnyView {
        let layout = RiseNativeMediaLayout::make(message, group, configuration, viewport_width, cx);
        RiseNativeMediaMessageCell::configure(reusing, message, group, layout.as_ref(), configuration, viewport_width, theme, cx)
    }
}
