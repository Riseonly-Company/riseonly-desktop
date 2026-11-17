use std::io::Cursor;
use std::path::PathBuf;
use std::slice;

use gpui::{DevicePixels, TestAppContext};
use image::codecs::jpeg::JpegEncoder;
use image::{Rgb, RgbImage};
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::media_viewer::media_viewer_image_cache::{MediaViewerBitmapLoader, MediaViewerImageMemoryCache};
use crate::core::ui::clever_image_ui::CleverImageCachePolicy;
use crate::modules::chat::components::chat::media_viewer::chat_media_gallery_builder::ChatMediaGalleryBuilder;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatInfo, ChatType};
use crate::modules::chat::stores::message::message_actions::message_types::{
    GetMessageMessage, MessageContentType, MessageMediaItem, MessageMediaKind,
};

fn direct_chat() -> ChatInfo {
    let mut chat = ChatInfo::new("chat-1".to_owned());
    chat.r#type = ChatType::Direct;
    chat
}

fn media_item(id: &str, kind: MessageMediaKind, url: &str, thumbnail_url: &str) -> MessageMediaItem {
    let mut media = MessageMediaItem::new(id.to_owned(), kind, url.to_owned());
    media.thumbnail_url = Some(thumbnail_url.to_owned());
    media.width = 1_920.0;
    media.height = 800.0;
    media
}

fn message(id: &str, content_type: MessageContentType, media: MessageMediaItem) -> GetMessageMessage {
    let mut message = GetMessageMessage::new(id.to_owned());
    message.chat_id = "chat-1".to_owned();
    message.sender_id = "sender-1".to_owned();
    message.content_type = content_type;
    message.media_items = vec![media];
    message
}

fn temporary_file(prefix: &str) -> PathBuf {
    std::env::temp_dir().join(format!("{prefix}-{}.jpg", Uuid::new_v4().uuid_string()))
}

struct MediaViewerLocalFileCleanup {
    local_url: PathBuf,
    source: String,
}

impl Drop for MediaViewerLocalFileCleanup {
    fn drop(&mut self) {
        MediaViewerImageMemoryCache::remove(&self.source, SaiAccountCacheScope::active_user_id().as_deref());
        let _ = std::fs::remove_file(&self.local_url);
    }
}

#[gpui::test]
fn chat_gallery_uses_the_completed_bubble_authoritative_display_source(cx: &mut TestAppContext) {
    let local_url = Url::from_file_path(&temporary_file("viewer-preview")).expect("temporary file url");
    let mut media = media_item(
        "media-1",
        MessageMediaKind::Image,
        "https://cdn.riseonly.net/public/chat/full.jpg",
        "https://cdn.riseonly.net/public/chat/thumb.jpg",
    );
    media.local_preview_url = Some(local_url.absolute_string().to_owned());
    let message = message("temp-message-1", MessageContentType::Image, media);
    let chat = direct_chat();

    let gallery = cx.update(|cx| {
        ChatMediaGalleryBuilder::items_messages(&chat, Some(slice::from_ref(&message)), &message, 0, cx)
    });
    let item = gallery.first().expect("gallery item");

    assert_eq!(item.display_source(), "https://cdn.riseonly.net/public/chat/thumb.jpg");
    assert_eq!(item.url, "https://cdn.riseonly.net/public/chat/full.jpg");
    assert_eq!(item.thumbnail_url.as_deref(), Some("https://cdn.riseonly.net/public/chat/thumb.jpg"));
    assert_eq!(item.width, 1_920.0);
    assert_eq!(item.height, 800.0);
    assert!((item.aspect_ratio() - 2.4).abs() < 0.0001);
}

#[gpui::test]
fn single_video_keeps_the_bubble_letterbox_geometry_for_hero_close(cx: &mut TestAppContext) {
    let media = media_item(
        "video-1",
        MessageMediaKind::Video,
        "https://cdn.riseonly.net/public/chat/video.mp4",
        "https://cdn.riseonly.net/public/chat/video.jpg",
    );
    let message = message("video-message", MessageContentType::Video, media);
    let chat = direct_chat();

    let gallery = cx.update(|cx| {
        ChatMediaGalleryBuilder::items_messages(&chat, Some(slice::from_ref(&message)), &message, 0, cx)
    });

    assert!(gallery.first().expect("gallery item").uses_fit_letterbox);
}

#[gpui::test]
async fn bitmap_loader_decodes_and_caches_a_file_url_for_pager_handoff(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let async_cx = cx.to_async();
    let image = RgbImage::from_pixel(320, 120, Rgb([88, 86, 214]));
    let mut data = Vec::new();
    JpegEncoder::new_with_quality(Cursor::new(&mut data), 90)
        .encode_image(&image)
        .expect("jpeg encode of a solid bitmap");
    let local_path = temporary_file("viewer-local");
    std::fs::write(&local_path, &data).expect("write of the local image");
    let local_url = Url::from_file_path(&local_path).expect("temporary file url");
    let source = local_url.absolute_string().to_owned();
    let _cleanup = MediaViewerLocalFileCleanup {
        local_url: local_path,
        source: source.clone(),
    };

    let resolved =
        MediaViewerBitmapLoader::load_into_memory(&source, true, false, CleverImageCachePolicy::MemoryOnly, &async_cx)
            .await;
    let bitmap = resolved.expect("decoded local file");
    let size = bitmap.size(0);

    assert_eq!(size.width, DevicePixels(320));
    assert_eq!(size.height, DevicePixels(120));
    assert!(
        MediaViewerImageMemoryCache::memory_only(&source, SaiAccountCacheScope::active_user_id().as_deref()).is_some()
    );
}
