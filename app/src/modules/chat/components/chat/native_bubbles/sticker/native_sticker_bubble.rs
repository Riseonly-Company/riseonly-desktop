use std::path::PathBuf;
use std::rc::Rc;

use gpui::{AnyView, App, Window};
use rise_chat::ChatType;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;
use rise_theme::AppTheme;

use crate::core::ui::lottie_sticker::sticker_premium_effect::StickerPremiumEffectRegistry;
use crate::modules::chat::components::chat::async_render::chat_message_render_snapshot::ChatMessageRenderEntryGroupFlags;
use crate::modules::chat::components::chat::async_render::rise_exact_fallback_geometry::RiseExactFallbackGeometry;
use crate::modules::chat::components::chat::async_render::rise_message_list_view::RiseMessageCollectionConfiguration;
use crate::modules::chat::components::chat::bubble::message_content::{MessageContent, MessageContentKind};
use crate::modules::chat::components::chat::native_bubbles::native_bubble_registry::NativeBubbleKind;
use crate::modules::chat::components::chat::native_bubbles::sticker::native_interactive_message_cell::NativeInteractiveMessageCell;
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatInfo;
use crate::modules::chat::stores::message::message_actions::message_types::{GetMessageMessage, MessageStickerKind};

/// static sticker priced without the hosted measure
/// picture with drop shadow and the time plate hanging past the bottom trailing corner
pub enum NativeStickerBubble {}

impl NativeBubbleKind for NativeStickerBubble {
    const REUSE_IDENTIFIER: &'static str = "NativeStickerBubble";

    fn row_height(
        message: &GetMessageMessage,
        group: ChatMessageRenderEntryGroupFlags,
        configuration: &RiseMessageCollectionConfiguration,
        viewport_width: f32,
        cx: &mut App,
    ) -> Option<f32> {
        if !Self::supports(message, &configuration.chat) {
            return None;
        }
        let entry = NativeInteractiveMessageCell::entry(message, group, configuration);
        RiseExactFallbackGeometry::row_height(&entry, configuration, viewport_width, cx)
    }

    fn make_view(
        reusing: Option<AnyView>,
        message: &GetMessageMessage,
        group: ChatMessageRenderEntryGroupFlags,
        configuration: &Rc<RiseMessageCollectionConfiguration>,
        viewport_width: f32,
        _theme: &AppTheme,
        _window: &mut Window,
        cx: &mut App,
    ) -> AnyView {
        let height = Self::row_height(message, group, configuration, viewport_width, cx);
        NativeInteractiveMessageCell::make_view(reusing, message, group, configuration, viewport_width, height, cx)
    }
}

impl NativeStickerBubble {
    /// shape taken, a static local picture with nothing beside or under it
    /// not moderated, in a direct chat or group
    /// animated or remote stay regular
    pub fn supports(message: &GetMessageMessage, chat: &ChatInfo) -> bool {
        if chat.r#type != ChatType::Direct && chat.r#type != ChatType::Group {
            return false;
        }
        if MessageContent::bubble_kind(message, Some(chat)) != MessageContentKind::Sticker {
            return false;
        }
        let Some(item) = message.first_sticker_item() else {
            return false;
        };
        matches!(item.sticker_kind, None | Some(MessageStickerKind::Static))
            && Self::local_file_url(&item.url).is_some()
            && StickerPremiumEffectRegistry::shared().effect_url(&item.url).is_none()
            && message.reply_to.is_none()
            && message.story_reply.is_none()
            && message.forward_info.is_none()
            && message.via_bot_label_tag().is_none()
            && !message.has_reaction_row()
            && !message.reply_markup.as_ref().is_some_and(|markup| markup.has_inline_keyboard())
            && message.discussion_chat_id.as_deref().is_none_or(str::is_empty)
            && !message.is_disabled_by_moderation()
    }

    /// image reads file:// and absolute paths from disk
    pub fn local_file_url(raw: &str) -> Option<PathBuf> {
        let source = trimming_whitespaces_and_newlines(raw);
        if source.starts_with("file://") {
            return Url::from_string(source).and_then(|url| url.to_file_path());
        }
        if source.starts_with('/') {
            return Some(PathBuf::from(source));
        }
        None
    }
}
