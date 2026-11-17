use std::path::{Path, PathBuf};

use rise_core::foundation::foundation_file_url::is_local_absolute_path;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;
use rise_i18n::tr;
use rise_platform::file_manager::FileManager;

use crate::modules::chat::stores::message::message_actions::message_types::{
    MessageContentType, MessageMediaItem, MessageMediaKind, MessageStickerKind,
};

const ANIMATION_LABEL: &str = "GIF";

/// what a quoted message looks like before it is quoted
/// composer bar and sent bubble quote share this so neither keeps its own idea of which
/// attachment is showable
pub enum MessageReplyPreview {}

impl MessageReplyPreview {
    /// first attachment that can be drawn as a still image
    /// thumbnail beats original, video voice and audio kinds only count through a real thumbnail
    pub fn thumbnail_source(items: &[MessageMediaItem]) -> Option<String> {
        for item in items {
            if let Some(thumbnail) = Self::usable_image_source(item.thumbnail_url.as_deref()) {
                return Some(thumbnail);
            }
            if let Some(local_preview) = Self::usable_image_source(item.resolved_local_preview_url().as_deref()) {
                return Some(local_preview);
            }
        }

        for item in items {
            let mime_is_image = item
                .mime_type
                .as_deref()
                .is_some_and(|mime| mime.to_lowercase().starts_with("image/"));
            let can_use_original = match item.kind {
                MessageMediaKind::Image | MessageMediaKind::Gif => true,
                MessageMediaKind::Animation => {
                    mime_is_image || ["gif", "apng", "webp"].contains(&Self::path_extension(&item.url).as_str())
                }
                MessageMediaKind::Sticker => {
                    item.sticker_kind == Some(MessageStickerKind::Static) || item.sticker_kind.is_none() || mime_is_image
                }
                MessageMediaKind::Document => mime_is_image,
                MessageMediaKind::Video
                | MessageMediaKind::VideoNote
                | MessageMediaKind::Audio
                | MessageMediaKind::Voice => false,
            };
            if can_use_original && let Some(source) = Self::usable_image_source(Some(item.display_source().as_str())) {
                return Some(source);
            }
        }
        None
    }

    /// line shown instead of text for a message with none
    /// next to a thumbnail the photo and video labels drop their emoji
    pub fn label(content_type: &MessageContentType, content: &str, has_thumbnail: bool) -> String {
        let is_poll_payload = *content_type == MessageContentType::Poll && content.trim_start().starts_with('{');
        if !content.is_empty() && !is_poll_payload {
            return content.to_owned();
        }
        match content_type {
            MessageContentType::Image => {
                tr(if has_thumbnail { "msg_preview_photo_plain" } else { "msg_reply_image" })
            }
            MessageContentType::Video => {
                tr(if has_thumbnail { "msg_preview_video_plain" } else { "msg_reply_video" })
            }
            MessageContentType::Voice => tr("msg_preview_voice"),
            MessageContentType::Audio => tr("msg_reply_audio"),
            MessageContentType::Document => tr("msg_reply_document"),
            MessageContentType::Sticker => tr("msg_preview_sticker"),
            MessageContentType::Animation => ANIMATION_LABEL.to_owned(),
            MessageContentType::VideoNote => tr("msg_reply_video_note"),
            MessageContentType::Contact => tr("msg_reply_contact"),
            MessageContentType::Location => tr("msg_reply_location"),
            MessageContentType::Poll => tr("msg_reply_poll"),
            MessageContentType::Call => tr("chat_call_generic"),
            _ => tr("msg_reply_generic"),
        }
    }

    fn usable_image_source(raw_value: Option<&str>) -> Option<String> {
        let value = trimming_whitespaces_and_newlines(raw_value?);
        if value.is_empty()
            || ["mp4", "mov", "m4v", "webm"].contains(&Self::path_extension(value).as_str())
            || !(value.to_lowercase().starts_with("http") || Self::is_drawable_local_file(value))
        {
            return None;
        }
        Some(value.to_owned())
    }

    fn is_drawable_local_file(value: &str) -> bool {
        let path: PathBuf = if value.starts_with("file://") {
            match Url::from_string(value).and_then(|url| url.to_file_path()) {
                Some(path) => path,
                None => return false,
            }
        } else if is_local_absolute_path(value) {
            PathBuf::from(value)
        } else {
            return false;
        };
        let mut is_directory = false;
        FileManager.file_exists_is_directory(&path, &mut is_directory) && !is_directory
    }

    fn path_extension(source: &str) -> String {
        if let Some(url) = Url::from_string(source) {
            let extension = url.path_extension();
            if !extension.is_empty() {
                return extension.to_lowercase();
            }
        }
        Path::new(source)
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_lowercase)
            .unwrap_or_default()
    }
}
