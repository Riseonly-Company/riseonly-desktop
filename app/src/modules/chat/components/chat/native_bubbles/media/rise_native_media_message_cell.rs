use std::rc::Rc;

use gpui::{AnyView, App};
use rise_theme::AppTheme;

use crate::modules::chat::components::chat::async_render::chat_message_render_snapshot::ChatMessageRenderEntryGroupFlags;
use crate::modules::chat::components::chat::async_render::rise_message_list_view::RiseMessageCollectionConfiguration;
use crate::modules::chat::components::chat::native_bubbles::media::rise_native_media_layout::RiseNativeMediaLayout;
use crate::modules::chat::components::chat::native_bubbles::sticker::native_interactive_message_cell::NativeInteractiveMessageCell;
use crate::modules::chat::stores::message::message_actions::message_types::GetMessageMessage;

/// native photo and video row
/// like other native kinds the row is the regular bubble row
/// laid out in the exact height the layout answers
pub enum RiseNativeMediaMessageCell {}

impl RiseNativeMediaMessageCell {
    pub const REUSE_IDENTIFIER: &'static str = "RiseNativeMediaMessageCell";

    #[allow(clippy::too_many_arguments)]
    pub fn configure(
        reusing: Option<AnyView>,
        message: &GetMessageMessage,
        group: ChatMessageRenderEntryGroupFlags,
        layout: Option<&RiseNativeMediaLayout>,
        configuration: &Rc<RiseMessageCollectionConfiguration>,
        viewport_width: f32,
        _theme: &AppTheme,
        cx: &mut App,
    ) -> AnyView {
        let height = layout.map(|layout| layout.row_height);
        NativeInteractiveMessageCell::make_view(reusing, message, group, configuration, viewport_width, height, cx)
    }
}
