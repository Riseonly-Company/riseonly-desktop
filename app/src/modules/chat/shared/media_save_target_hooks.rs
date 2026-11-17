use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;

use crate::core::media::rise_media_kind_classifier::{RiseMediaKind, RiseMediaKindClassifier};
use crate::modules::chat::stores::message::message_actions::message_types::{MessageMediaItem, MessageMediaKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaSaveTargetKind {
    Gallery,
    Files,
}

/// decides where an attachment gets saved
/// pictures and movies go to the photo library and everything else to a folder
pub enum MessageMediaSaveClassifier {}

impl MessageMediaSaveClassifier {
    pub fn target(item: &MessageMediaItem) -> Option<MediaSaveTargetKind> {
        if item.is_cancelled || item.is_uploading {
            return None;
        }
        if trimming_whitespaces_and_newlines(&item.url).is_empty() {
            return None;
        }

        match item.kind {
            MessageMediaKind::Sticker => None,
            MessageMediaKind::Image
            | MessageMediaKind::Animation
            | MessageMediaKind::Gif
            | MessageMediaKind::Video
            | MessageMediaKind::VideoNote => Some(MediaSaveTargetKind::Gallery),
            MessageMediaKind::Audio | MessageMediaKind::Voice => Some(MediaSaveTargetKind::Files),
            MessageMediaKind::Document => {
                if Self::is_video(item) || Self::is_image(item) {
                    Some(MediaSaveTargetKind::Gallery)
                } else {
                    Some(MediaSaveTargetKind::Files)
                }
            }
        }
    }

    pub fn is_video(item: &MessageMediaItem) -> bool {
        match item.kind {
            MessageMediaKind::Video | MessageMediaKind::VideoNote => true,
            MessageMediaKind::Image
            | MessageMediaKind::Animation
            | MessageMediaKind::Gif
            | MessageMediaKind::Audio
            | MessageMediaKind::Voice
            | MessageMediaKind::Sticker => false,
            MessageMediaKind::Document => Self::document_kind(item) == RiseMediaKind::Video,
        }
    }

    fn is_image(item: &MessageMediaItem) -> bool {
        Self::document_kind(item) == RiseMediaKind::Image
    }

    fn document_kind(item: &MessageMediaItem) -> RiseMediaKind {
        RiseMediaKindClassifier::kind_file_extension(
            item.mime_type.as_deref(),
            &RiseMediaKindClassifier::first_file_extension(&[item.file_name.as_deref(), Some(item.url.as_str())]),
        )
    }
}
