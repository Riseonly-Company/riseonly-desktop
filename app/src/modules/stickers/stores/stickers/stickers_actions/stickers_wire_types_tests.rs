use gpui::TestAppContext;
use serde_json::Value;

use crate::modules::chat::components::chat::bubble::message_sticker_view::{
    ChatAnimatedMediaPlaybackKind, ChatAnimatedMediaSourcePolicy,
};
use crate::modules::chat::stores::message::message_actions::message_types::MessageStickerKind;
use crate::modules::chat::stores::message::message_services::message_services::MessageServicesStore;
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::{StickerAssetType, StickerItem};
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_wire_types::{
    StickerWireDTO, StickerWireDTOExt,
};

fn decode_sticker(payload: &str) -> StickerItem {
    serde_json::from_str::<StickerWireDTO>(payload).expect("sticker payload decodes").to_sticker()
}

#[test]
fn sticker_wire_metadata_survives_model_mapping() {
    let item = decode_sticker(
        r#"{
          "id": "sticker-1",
          "pack_id": "pack-1",
          "file_url": "https://cdn.example/signed?id=1",
          "asset_type": "video",
          "mime_type": "video/webm",
          "codec": "vp9"
        }"#,
    );

    assert_eq!(item.mime_type.as_deref(), Some("video/webm"));
    assert_eq!(item.codec.as_deref(), Some("vp9"));
    assert_eq!(item.asset_type, StickerAssetType::Video);
    let row = item.to_raw_row();
    assert_eq!(row.get("mime_type").and_then(Value::as_str), Some("video/webm"));
    assert_eq!(row.get("codec").and_then(Value::as_str), Some("vp9"));
}

#[test]
fn legacy_sticker_wire_payload_without_media_metadata_still_decodes() {
    let item = decode_sticker(
        r#"{
          "id": "legacy-sticker",
          "pack_id": "legacy-pack",
          "file_url": "https://cdn.example/legacy-sticker.webp",
          "asset_type": "static"
        }"#,
    );

    assert_eq!(item.id, "legacy-sticker");
    assert_eq!(item.mime_type, None);
    assert_eq!(item.codec, None);
    assert_eq!(item.asset_type, StickerAssetType::StaticImage);
}

#[test]
fn legacy_bunny_sticker_urls_use_public_cdn_at_model_boundary() {
    let item = decode_sticker(
        r#"{
          "id": "server-lottie",
          "pack_id": "pack-1",
          "file_url": "https://de-s3.storage.bunnycdn.com/riseonly-s3/public%2Fstickers%2Fasset-sticker.ros",
          "thumbnail_url": "https://storage.bunnycdn.com/riseonly-s3/public%2Fstickers%2Fasset-preview.webp",
          "asset_type": "lottie",
          "mime_type": "application/x-rosticker"
        }"#,
    );

    assert_eq!(item.file_url, "https://cdn.riseonly.net/public/stickers/asset-sticker.ros");
    assert_eq!(item.thumbnail_url.as_deref(), Some("https://cdn.riseonly.net/public/stickers/asset-preview.webp"));
    assert_eq!(item.asset_type, StickerAssetType::Lottie);
}

#[gpui::test]
fn create_message_carries_sticker_mime_type_and_codec(cx: &mut TestAppContext) {
    let sticker = StickerItem::new(
        "sticker-2".to_owned(),
        "pack-2".to_owned(),
        "https://cdn.example/signed?id=2".to_owned(),
        None,
        512,
        512,
        Some(2_000),
        0,
        0,
        vec!["\u{1F642}".to_owned()],
        StickerAssetType::Video,
        Some("video/webm".to_owned()),
        Some("vp09.00.10.08".to_owned()),
    );

    let body = cx.update(|cx| {
        MessageServicesStore::shared(cx).read(cx).build_create_message_body_for_sticker(
            &sticker, None, None, "chat-1", None,
        )
    });
    let media = body.media_items.as_ref().and_then(|items| items.first()).expect("sticker media item");

    assert_eq!(media.mime_type.as_deref(), Some("video/webm"));
    assert_eq!(media.codec.as_deref(), Some("vp09.00.10.08"));
    assert_eq!(media.sticker_type.as_deref(), Some(MessageStickerKind::Animated.raw_value()));

    let encoded = serde_json::to_value(&body).expect("body encodes");
    let encoded_media = encoded
        .get("media_items")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .expect("encoded media item");
    assert_eq!(encoded_media.get("mime_type").and_then(Value::as_str), Some("video/webm"));
    assert_eq!(encoded_media.get("codec").and_then(Value::as_str), Some("vp09.00.10.08"));
}

#[test]
fn signed_extensionless_video_uses_codec_family_deterministically() {
    let signed_url = "https://cdn.example/media/signed?id=42";

    for codec in ["h264", "AVC1.640028", "hevc", "hvc1.1.6.L93.B0"] {
        assert_eq!(
            ChatAnimatedMediaSourcePolicy::playback_kind(signed_url, None, Some(codec)),
            ChatAnimatedMediaPlaybackKind::AvFoundationVideo
        );
    }

    assert_eq!(
        ChatAnimatedMediaSourcePolicy::playback_kind(signed_url, None, Some("vp8")),
        ChatAnimatedMediaPlaybackKind::UnsupportedVideo
    );
    for codec in ["VP9", "vp09.00.10.08"] {
        assert_eq!(
            ChatAnimatedMediaSourcePolicy::playback_kind(signed_url, None, Some(codec)),
            ChatAnimatedMediaPlaybackKind::WebMVideo
        );
    }

    assert_eq!(
        ChatAnimatedMediaSourcePolicy::playback_kind(signed_url, Some("video/mp4"), Some("vp9")),
        ChatAnimatedMediaPlaybackKind::WebMVideo
    );
    assert_eq!(
        ChatAnimatedMediaSourcePolicy::playback_kind(signed_url, Some("video/webm"), Some("h264")),
        ChatAnimatedMediaPlaybackKind::WebMVideo
    );
    assert_eq!(
        ChatAnimatedMediaSourcePolicy::playback_kind(signed_url, Some("video/webm"), Some("vp8")),
        ChatAnimatedMediaPlaybackKind::UnsupportedVideo
    );
}
