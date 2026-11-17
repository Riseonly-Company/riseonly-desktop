use std::fs;

use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;

use crate::core::ui::animated_gif_view::{AnimatedImageSourcePolicy, AnimatedImageSourcePolicySource};
use crate::modules::chat::stores::message::message_actions::message_types::{
    GetMessageMessage, MessageContentType, MessageMediaItem, MessageMediaKind,
};
use crate::modules::chat::stores::message::message_services::message_services::MessageServicesStore;

fn message(id: &str, media: MessageMediaItem) -> GetMessageMessage {
    let mut message = GetMessageMessage::new(id.to_owned());
    message.content_type = MessageContentType::Animation;
    message.media_items = vec![media];
    message
}

fn animation(id: &str, url: &str, mime_type: &str) -> MessageMediaItem {
    let mut item = MessageMediaItem::new(id.to_owned(), MessageMediaKind::Animation, url.to_owned());
    item.mime_type = Some(mime_type.to_owned());
    item
}

fn optimistic_animation(id: &str, local_preview: &str) -> MessageMediaItem {
    let mut item = animation(id, local_preview, "image/gif");
    item.local_preview_url = Some(local_preview.to_owned());
    item
}

fn first_preview(message: &GetMessageMessage) -> Option<&str> {
    message.media_items.first().and_then(|item| item.local_preview_url.as_deref())
}

#[test]
fn file_url_and_absolute_path_use_bounded_mapped_data() {
    let directory = std::env::temp_dir().join(format!("animated-gif-source-policy-{}", Uuid::new_v4().uuid_string()));
    assert!(fs::create_dir_all(&directory).is_ok());

    let file_path = directory.join("local preview.gif");
    let payload: Vec<u8> = vec![0x47, 0x49, 0x46, 0x38, 0x39, 0x61];
    assert!(fs::write(&file_path, &payload).is_ok());
    let file_url = Url::from_file_path(&file_path).map(|url| url.absolute_string().to_owned()).unwrap_or_default();
    let raw_path = file_path.to_string_lossy().into_owned();

    assert_eq!(
        AnimatedImageSourcePolicy::resolve(&file_url),
        Some(AnimatedImageSourcePolicySource::LocalFile(file_path.clone()))
    );
    assert_eq!(
        AnimatedImageSourcePolicy::resolve(&raw_path),
        Some(AnimatedImageSourcePolicySource::LocalFile(file_path.clone()))
    );
    assert_eq!(AnimatedImageSourcePolicy::mapped_local_data(&file_path, payload.len() as i64), Some(payload.clone()));
    assert_eq!(AnimatedImageSourcePolicy::mapped_local_data(&file_path, payload.len() as i64 - 1), None);
    assert_eq!(
        AnimatedImageSourcePolicy::resolve("https://cdn.example/remote.gif"),
        Some(AnimatedImageSourcePolicySource::Remote)
    );
    assert_eq!(AnimatedImageSourcePolicy::resolve("relative.gif"), None);

    let _ = fs::remove_dir_all(&directory);
}

#[test]
fn send_success_drops_supported_preview_but_keeps_undecodable_web_m() {
    let local_path = std::env::temp_dir().join(format!("optimistic-preview-{}.gif", Uuid::new_v4().uuid_string()));
    assert!(fs::write(&local_path, [0x47_u8, 0x49, 0x46]).is_ok());
    let local_preview = Url::from_file_path(&local_path).map(|url| url.absolute_string().to_owned()).unwrap_or_default();

    let mut kept = message("temp-gif", optimistic_animation("optimistic-gif", &local_preview));
    let real = message("server-gif", animation("remote-gif", "https://cdn.example/remote.gif", "image/gif"));

    MessageServicesStore::merge_real_message_fields(&real, &mut kept, "temp-gif");

    assert_eq!(first_preview(&kept), None);
    assert_eq!(kept.media_items.first().map(|item| item.url.as_str()), Some("https://cdn.example/remote.gif"));

    let mut playable_web_m = message("temp-vp9", optimistic_animation("optimistic-vp9", &local_preview));
    MessageServicesStore::merge_real_message_fields(
        &message("server-vp9", animation("remote-vp9", "https://cdn.example/remote.webm", "video/webm")),
        &mut playable_web_m,
        "temp-vp9",
    );
    assert_eq!(first_preview(&playable_web_m), None);

    let mut kept_web_m = message("temp-webm", optimistic_animation("optimistic-webm", &local_preview));
    let mut vp8 = animation("remote-webm", "https://cdn.example/remote.webm", "video/webm");
    vp8.codec = Some("vp8".to_owned());
    let real_web_m = message("server-webm", vp8);

    MessageServicesStore::merge_real_message_fields(&real_web_m, &mut kept_web_m, "temp-webm");

    assert_eq!(first_preview(&kept_web_m), Some(local_preview.as_str()));

    let _ = fs::remove_file(&local_path);
}
