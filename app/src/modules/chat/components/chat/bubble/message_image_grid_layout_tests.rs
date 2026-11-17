use gpui::{Bounds, point, px, size};
use rise_canon::rise_key::message;
use serde_json::{Map, Value};

use crate::modules::chat::components::chat::bubble::message_image_grid_layout::{MessageGridLayout, MessageGridLayoutEngine};
use crate::modules::chat::components::chat::composer::chat_composer_types::{
    ChatComposerCustomEmojiAttachmentPayload, ChatComposerCustomEmojiEditHydrator,
};
use crate::modules::chat::shared::config::message_sizes::MessageSizes;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatInfo, ChatType};
use crate::modules::chat::stores::message::message_actions::message_types::{
    MessageContentType, MessageCustomEmojiEntity, MessageMediaItem, MessageMediaKind, MessageStickerKind,
    RemoteMediaURL,
};
use crate::modules::chat::stores::message::message_actions::message_wire_types::{
    EditMessageBody, MessageMediaInfoWireDTO, MessagePayloadCodec,
};
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::{StickerAssetType, StickerItem};

struct MessageGridLayoutTests;

fn rect(x: f32, y: f32, width: f32, height: f32) -> Bounds<gpui::Pixels> {
    Bounds::new(point(px(x), px(y)), size(px(width), px(height)))
}

fn value(pixels: gpui::Pixels) -> f32 {
    f32::from(pixels)
}

fn chat(id: &str, chat_type: ChatType) -> ChatInfo {
    let mut chat = ChatInfo::new(id.to_owned());
    chat.r#type = chat_type;
    chat
}

fn item(id: &str, kind: MessageMediaKind, url: &str, width: f32, height: f32) -> MessageMediaItem {
    let mut item = MessageMediaItem::new(id.to_owned(), kind, url.to_owned());
    item.width = width;
    item.height = height;
    item
}

fn media(id: &str, width: f32, height: f32) -> MessageMediaItem {
    item(id, MessageMediaKind::Image, &format!("https://example.com/{id}.jpg"), width, height)
}

fn compute(items: &[MessageMediaItem], max_width: f32, max_row_height: f32) -> MessageGridLayout {
    MessageGridLayoutEngine::compute(items, max_width, max_row_height, 1.0, None)
}

#[test]
fn incoming_group_media_uses_only_the_available_row_width() {
    let chat = chat("group", ChatType::Group);

    let incoming = MessageSizes::available_bubble_width(375.0, &chat, false);
    let outgoing = MessageSizes::available_bubble_width(375.0, &chat, true);

    assert!((incoming - 292.0).abs() <= 0.001);
    assert!((outgoing - 330.0).abs() <= 0.001);
}

#[test]
fn bubble_widths_track_telegram_compact_and_regular_breakpoints() {
    let direct = chat("direct", ChatType::Direct);
    let group = chat("group", ChatType::Group);

    assert_eq!(MessageSizes::available_bubble_width(320.0, &direct, true), 275.0);
    assert_eq!(MessageSizes::available_bubble_width(390.0, &group, false), 307.0);
    assert_eq!(MessageSizes::available_bubble_width(768.0, &direct, true), 490.0);
}

#[test]
fn single_panoramic_animation_is_aspect_sized_and_bounded() {
    let item = item("animation", MessageMediaKind::Animation, "https://example.com/a.gif", 1_920.0, 1_080.0);

    let layout = compute(&[item], 261.0, 380.0);
    let cell = layout.cells.first().expect("one cell");

    assert!(layout.is_single);
    assert!((value(layout.container_size.width) - 261.0).abs() <= 0.001);
    assert!((value(layout.container_size.height) - 146.0).abs() <= 0.001);
    assert!(value(cell.cell_frame.right()) <= value(layout.container_size.width) + 0.001);
    assert!(value(cell.cell_frame.bottom()) <= value(layout.container_size.height) + 0.001);
    assert!(value(cell.image_frame.left()) >= value(cell.cell_frame.left()) - 0.001);
    assert!(value(cell.image_frame.top()) >= value(cell.cell_frame.top()) - 0.001);
    assert!(value(cell.image_frame.right()) <= value(cell.cell_frame.right()) + 0.001);
    assert!(value(cell.image_frame.bottom()) <= value(cell.cell_frame.bottom()) + 0.001);
}

#[test]
fn single_portrait_media_uses_telegram_minimum_width_and_height_cap() {
    let item = item("portrait", MessageMediaKind::Gif, "https://example.com/p.gif", 600.0, 1_800.0);

    let layout = compute(&[item], 302.0, 380.0);
    let cell = layout.cells.first().expect("one cell");

    assert!((value(layout.container_size.width) - 170.0).abs() <= 0.001);
    assert!((value(layout.container_size.height) - 380.0).abs() <= 0.001);
    assert!((value(cell.image_frame.size.width) - 126.0).abs() <= 0.001);
    assert!((value(cell.image_frame.left()) - 22.0).abs() <= 0.001);
    assert!(value(cell.image_frame.left()) >= value(cell.cell_frame.left()) - 0.001);
    assert!(value(cell.image_frame.right()) <= value(cell.cell_frame.right()) + 0.001);
    assert!(value(cell.image_frame.top()) >= value(cell.cell_frame.top()) - 0.001);
    assert!(value(cell.image_frame.bottom()) <= value(cell.cell_frame.bottom()) + 0.001);
}

#[test]
fn minimum_bubble_width_never_exceeds_the_available_width() {
    let item = item("narrow", MessageMediaKind::Animation, "https://example.com/n.gif", 100.0, 100.0);

    let layout = compute(&[item], 80.0, 380.0);

    assert!((value(layout.container_size.width) - 80.0).abs() <= 0.001);
}

#[test]
fn regular_width_media_uses_telegram_tablet_bounds() {
    let item = item("regular", MessageMediaKind::Image, "https://example.com/regular.jpg", 1_600.0, 1_600.0);

    let layout = compute(&[item], 440.0, 440.0);
    let cell = layout.cells.first().expect("one cell");

    assert_eq!(layout.container_size, size(px(440.0), px(440.0)));
    assert_eq!(cell.cell_frame, rect(0.0, 0.0, 440.0, 440.0));
}

#[test]
fn album_container_contains_every_cell() {
    let items = [
        item("one", MessageMediaKind::Image, "https://example.com/1.jpg", 1_600.0, 900.0),
        item("two", MessageMediaKind::Image, "https://example.com/2.jpg", 900.0, 1_600.0),
    ];

    let layout = compute(&items, 260.0, 380.0);

    assert!(layout.cells.iter().all(|cell| {
        value(cell.cell_frame.right()) <= value(layout.container_size.width) + 0.001
            && value(cell.cell_frame.bottom()) <= value(layout.container_size.height) + 0.001
    }));
}

#[test]
fn mosaic_uses_exactly_the_requested_physical_pixel_separator() {
    let pixel: f32 = 1.0 / 3.0;
    let horizontal = MessageGridLayoutEngine::compute(
        &[media("left", 1_000.0, 1_000.0), media("right", 1_000.0, 1_000.0)],
        300.0,
        380.0,
        pixel,
        None,
    );
    let left = horizontal.cells.first().expect("left cell");
    let right = horizontal.cells.last().expect("right cell");
    assert!((value(right.cell_frame.left()) - value(left.cell_frame.right()) - pixel).abs() <= 0.001);

    let vertical = MessageGridLayoutEngine::compute(
        &[media("top", 1_500.0, 1_000.0), media("bottom", 1_500.0, 1_000.0)],
        300.0,
        380.0,
        pixel,
        None,
    );
    let top = vertical.cells.first().expect("top cell");
    let bottom = vertical.cells.last().expect("bottom cell");
    assert!((value(bottom.cell_frame.top()) - value(top.cell_frame.bottom()) - pixel).abs() <= 0.001);
}

#[test]
fn two_similar_landscape_items_stack_vertically() {
    let items = [media("one", 1_500.0, 1_000.0), media("two", 1_500.0, 1_000.0)];

    let layout = compute(&items, 300.0, 380.0);
    let first = layout.cells.first().expect("first cell");
    let second = layout.cells.last().expect("second cell");

    assert_eq!(first.cell_frame, rect(0.0, 0.0, 300.0, 189.0));
    assert_eq!(second.cell_frame, rect(0.0, 190.0, 300.0, 189.0));
    assert_eq!(layout.container_size, size(px(300.0), px(379.0)));
}

#[test]
fn portrait_led_three_item_mosaic_uses_one_tall_and_two_stacked_cells() {
    let items = [
        media("portrait", 500.0, 1_000.0),
        media("top-right", 1_000.0, 1_000.0),
        media("bottom-right", 1_000.0, 1_000.0),
    ];

    let layout = compute(&items, 300.0, 380.0);

    assert_eq!(layout.cells[0].cell_frame, rect(0.0, 0.0, 150.0, 380.0));
    assert_eq!(layout.cells[1].cell_frame, rect(151.0, 0.0, 149.5, 229.0));
    assert_eq!(layout.cells[2].cell_frame, rect(151.0, 230.0, 149.5, 150.0));
    assert_eq!(layout.container_size, size(px(301.0), px(380.0)));
}

#[test]
fn landscape_led_four_item_mosaic_uses_wide_header_and_three_footer_cells() {
    let items: Vec<MessageMediaItem> = (0..4).map(|index| media(&format!("item-{index}"), 1_500.0, 1_000.0)).collect();

    let layout = compute(&items, 300.0, 380.0);

    assert_eq!(layout.cells[0].cell_frame, rect(0.0, 0.0, 300.0, 200.0));
    assert_eq!(layout.cells[1].cell_frame, rect(0.0, 201.0, 99.0, 81.0));
    assert_eq!(layout.cells[2].cell_frame, rect(100.0, 201.0, 100.0, 81.0));
    assert_eq!(layout.cells[3].cell_frame, rect(201.0, 201.0, 99.0, 81.0));
    assert_eq!(layout.container_size, size(px(300.0), px(282.0)));
}

#[test]
fn five_item_optimizer_is_deterministic_and_keeps_telegram_rounding() {
    let items: Vec<MessageMediaItem> = (0..5).map(|index| media(&format!("square-{index}"), 1_000.0, 1_000.0)).collect();

    let first = compute(&items, 300.0, 380.0);
    let second = compute(&items, 300.0, 380.0);

    assert_eq!(first, second);
    assert_eq!(
        first.rows.iter().map(|row| row.cell_indices.clone()).collect::<Vec<_>>(),
        vec![vec![0, 1], vec![2, 3, 4]]
    );
    assert_eq!(first.container_size, size(px(302.0), px(251.0)));
    assert_eq!(
        value(first.container_size.width) - value(MessageGridLayoutEngine::COMPACT_MAXIMUM_SIZE.width),
        2.0
    );
    assert!(first.cells.iter().all(|cell| {
        cell.cell_frame.right() <= first.container_size.width && cell.cell_frame.bottom() <= first.container_size.height
    }));
}

#[test]
fn scaled_mosaic_rounds_its_container_like_telegram() {
    let items = [
        media("portrait", 500.0, 1_000.0),
        media("top-right", 1_000.0, 1_000.0),
        media("bottom-right", 1_000.0, 1_000.0),
    ];

    let layout = compute(&items, 282.0, 380.0);

    assert_eq!(value(layout.container_size.height), 357.0);
}

#[test]
fn narrow_synthetic_mosaic_never_produces_negative_frames() {
    let items: Vec<MessageMediaItem> = (0..4).map(|index| media(&format!("narrow-{index}"), 1_500.0, 1_000.0)).collect();

    let layout = compute(&items, 80.0, 380.0);

    assert!(layout.cells.iter().all(|cell| {
        value(cell.cell_frame.size.width) > 0.0
            && value(cell.cell_frame.size.height) > 0.0
            && value(cell.cell_frame.left()).is_finite()
            && value(cell.cell_frame.top()).is_finite()
    }));
}

#[test]
fn media_render_identity_stays_stable_across_authoritative_ack() {
    let mut optimistic = item("temp-media-0", MessageMediaKind::Video, "ph://asset-42", 1_920.0, 1_080.0);
    optimistic.thumbnail_url = Some("ph://asset-42".to_owned());
    optimistic.is_uploading = true;
    optimistic.local_slot_index = 0;

    let mut authoritative = optimistic.clone();
    authoritative.id = "server-file-9".to_owned();
    authoritative.url = "https://cdn.riseonly.net/public/chat/video.mp4".to_owned();
    authoritative.thumbnail_url = Some("https://cdn.riseonly.net/public/chat/poster.jpg".to_owned());
    authoritative.local_preview_url = Some("ph://asset-42".to_owned());
    authoritative.is_uploading = false;

    let before = MessageGridLayoutEngine::compute(&[optimistic], 300.0, 380.0, 1.0, Some("temp-message-7"));
    let after = MessageGridLayoutEngine::compute(&[authoritative.clone()], 300.0, 380.0, 1.0, Some("temp-message-7"));

    let before_cell = before.cells.first().expect("before cell");
    let after_cell = after.cells.first().expect("after cell");
    assert_ne!(before_cell.media_item_id, after_cell.media_item_id);
    assert_eq!(before_cell.id(), after_cell.id());
    assert_eq!(before_cell.render_id, after_cell.render_id);
    assert_eq!(before_cell.display_source, "ph://asset-42");
    assert_eq!(Some(after_cell.display_source.clone()), authoritative.thumbnail_url);
    assert_eq!(before_cell.cell_frame, after_cell.cell_frame);
    assert_eq!(before_cell.image_frame, after_cell.image_frame);
}

#[test]
fn completed_visual_media_replaces_local_previews_without_changing_identity() {
    for kind in [
        MessageMediaKind::Image,
        MessageMediaKind::Video,
        MessageMediaKind::VideoNote,
        MessageMediaKind::Animation,
        MessageMediaKind::Gif,
    ] {
        let mut item = item("media-1", kind, "https://example.com/media", 1_920.0, 1_080.0);
        item.thumbnail_url = Some("https://example.com/poster.jpg".to_owned());
        item.local_preview_url = Some("ph://picker-preview".to_owned());
        item.is_uploading = true;
        let render_id = item.stable_render_id("message-1");
        assert_eq!(item.display_source(), "ph://picker-preview");

        item.is_uploading = false;
        item.is_compressing = true;
        assert_eq!(item.display_source(), "ph://picker-preview");

        item.is_compressing = false;
        assert_eq!(item.display_source(), "https://example.com/poster.jpg");
        assert_eq!(item.stable_render_id("message-1"), render_id);

        item.thumbnail_url = Some("ph://picker-preview".to_owned());
        assert_eq!(item.display_source(), "https://example.com/media");

        item.url = "ph://picker-preview".to_owned();
        assert_eq!(item.display_source(), "ph://picker-preview");
    }
}

#[test]
fn filtered_grid_keeps_original_upload_slot_indices() {
    let mut first = media("slot-0", 1_000.0, 1_000.0);
    first.local_slot_index = 0;
    let mut third = media("slot-2", 1_000.0, 1_000.0);
    third.local_slot_index = 2;
    let layout = MessageGridLayoutEngine::compute(&[first, third], 300.0, 380.0, 1.0, Some("temp-message-8"));

    assert_eq!(layout.cells.iter().map(|cell| cell.upload_slot_index).collect::<Vec<_>>(), vec![0, 2]);
    assert_eq!(
        layout.cells.iter().map(|cell| cell.render_id.clone()).collect::<Vec<_>>(),
        vec!["14:temp-message-8:media:0".to_owned(), "14:temp-message-8:media:2".to_owned()]
    );
}

#[test]
fn photo_library_video_can_render_before_file_materialization() {
    let mut item = item("temp-media-0", MessageMediaKind::Video, "ph://asset-99", 1_920.0, 1_080.0);
    item.thumbnail_url = Some("ph://asset-99".to_owned());
    item.is_uploading = true;

    let layout = MessageGridLayoutEngine::compute(&[item.clone()], 300.0, 380.0, 1.0, Some("temp-message-9"));

    assert!(MessageMediaItem::is_local_preview_url(&item.url));
    assert_eq!(item.display_source(), "ph://asset-99");
    assert!(layout.cells.first().and_then(|cell| cell.upload_seed).is_some());
    assert_eq!(layout.cells.first().map(|cell| cell.display_source.as_str()), Some("ph://asset-99"));
}

fn wire_payload(entries: &[(&str, Value)]) -> Value {
    let mut payload = Map::new();
    for (key, value) in entries {
        payload.insert((*key).to_owned(), value.clone());
    }
    Value::Object(payload)
}

#[test]
fn incoming_gif_wire_keeps_aspect_metadata_and_normalizes_kind() {
    let payload = wire_payload(&[
        (message::MEDIA_TYPE, Value::from("gif")),
        (message::FILE_URL, Value::from("https://example.com/wire.gif")),
        (message::WIDTH, Value::from(1_280)),
        (message::HEIGHT, Value::from(720)),
    ]);
    let wire: MessageMediaInfoWireDTO = MessagePayloadCodec::decode(Some(&payload)).expect("decodes");
    let item = wire.to_media_item(0, "message", Some(MessageContentType::Animation));

    assert_eq!(item.kind, MessageMediaKind::Animation);
    assert_eq!(item.width, 1_280.0);
    assert_eq!(item.height, 720.0);
    assert!((item.aspect_ratio() - 16.0 / 9.0).abs() <= 0.001);
}

#[test]
fn ros_sticker_without_sticker_type_uses_lottie_and_public_cdn() {
    let payload = wire_payload(&[
        (message::MEDIA_TYPE, Value::from("sticker")),
        (
            message::FILE_URL,
            Value::from("https://de-s3.storage.bunnycdn.com/riseonly-s3/public%2Fstickers%2Fasset.ros"),
        ),
        (message::MIME_TYPE, Value::from("application/x-rosticker")),
    ]);
    let wire: MessageMediaInfoWireDTO = MessagePayloadCodec::decode(Some(&payload)).expect("decodes");
    let item = wire.to_media_item(0, "message", Some(MessageContentType::Sticker));

    assert_eq!(item.kind, MessageMediaKind::Sticker);
    assert_eq!(item.sticker_kind, Some(MessageStickerKind::Lottie));
    assert_eq!(item.url, "https://cdn.riseonly.net/public/stickers/asset.ros");
}

#[test]
fn document_owner_overrides_video_media_type_from_server() {
    let payload = wire_payload(&[
        (message::MEDIA_TYPE, Value::from("video")),
        (message::FILE_URL, Value::from("https://cdn.riseonly.net/public/chat/file.mp4")),
        (message::THUMBNAIL_URL, Value::from("https://cdn.riseonly.net/public/chat/thumb.jpg")),
        (message::MIME_TYPE, Value::from("video/mp4")),
    ]);
    let wire: MessageMediaInfoWireDTO = MessagePayloadCodec::decode(Some(&payload)).expect("decodes");
    let item = wire.to_media_item(0, "message", Some(MessageContentType::Document));

    assert_eq!(item.kind, MessageMediaKind::Document);
}

#[test]
fn public_bunny_storage_thumbnail_uses_riseonly_cdn() {
    let source = "https://de-s3.storage.bunnycdn.com/riseonly-s3/public%2Fchat%2Fthumb_file.jpg";

    assert_eq!(
        RemoteMediaURL::optional_normalized(Some(source)),
        Some("https://cdn.riseonly.net/public/chat/thumb_file.jpg".to_owned())
    );
}

#[test]
fn private_bunny_storage_url_is_not_promoted_to_public_cdn() {
    let source = "https://de-s3.storage.bunnycdn.com/riseonly-s3/private%2Fmusic%2Ftrack.mp3";

    assert_eq!(RemoteMediaURL::optional_normalized(Some(source)), Some(source.to_owned()));
}

#[test]
fn custom_emoji_edit_hydration_replaces_entity_ranges_and_rebases_offsets() {
    let hydration = ChatComposerCustomEmojiEditHydrator::hydrate(
        "aTOKENbXXc",
        &[
            MessageCustomEmojiEntity {
                offset: 1,
                length: 5,
                custom_emoji_id: "emoji-a".to_owned(),
                file_url: "https://cdn.riseonly.net/a.ros".to_owned(),
                asset_type: "lottie".to_owned(),
                width: 100,
                height: 100,
                pack_id: String::new(),
            },
            MessageCustomEmojiEntity {
                offset: 7,
                length: 2,
                custom_emoji_id: "emoji-b".to_owned(),
                file_url: "https://cdn.riseonly.net/b.webp".to_owned(),
                asset_type: "static".to_owned(),
                width: 100,
                height: 100,
                pack_id: String::new(),
            },
        ],
    );

    assert_eq!(hydration.text, "a\u{FFFC}b\u{FFFC}c");
    assert_eq!(hydration.entities.iter().map(|entity| entity.offset).collect::<Vec<_>>(), vec![1, 3]);
    assert_eq!(hydration.entities.iter().map(|entity| entity.length).collect::<Vec<_>>(), vec![1, 1]);
    assert_eq!(
        hydration.entities.iter().map(|entity| entity.custom_emoji_id.clone()).collect::<Vec<_>>(),
        vec!["emoji-a".to_owned(), "emoji-b".to_owned()]
    );
}

#[test]
fn custom_emoji_attachment_payload_survives_round_trip() {
    let item = StickerItem::new(
        "emoji-a".to_owned(),
        "pack-a".to_owned(),
        "https://cdn.riseonly.net/a.ros".to_owned(),
        Some("https://cdn.riseonly.net/a.webp".to_owned()),
        128,
        96,
        Some(1_250),
        7,
        42,
        vec!["\u{1F525}".to_owned()],
        StickerAssetType::Lottie,
        Some("application/x-rosticker".to_owned()),
        Some("lottie".to_owned()),
    );
    let encoded = ChatComposerCustomEmojiAttachmentPayload::new(&item)
        .encoded_string()
        .expect("encodes");
    let decoded = ChatComposerCustomEmojiAttachmentPayload::decode(Some(encoded.as_str())).expect("decodes");

    assert_eq!(decoded.item(), item);
}

#[test]
fn edit_message_body_encodes_explicit_empty_entities() {
    let body = EditMessageBody {
        caption: None,
        content: Some("updated".to_owned()),
        document: None,
        entities: Some("[]".to_owned()),
        media_items: None,
        message_id: "message".to_owned(),
        reply_markup: None,
        web_app_data: None,
    };
    let object = serde_json::to_value(&body).expect("encodes");

    assert_eq!(object.get(message::ENTITIES).and_then(Value::as_str), Some("[]"));
}
