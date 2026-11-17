use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};

use gpui::{Context, TestAppContext, bounds, point, size};
use image::codecs::jpeg::JpegEncoder;
use image::{DynamicImage, ExtendedColorType, ImageFormat, Rgb, RgbImage, Rgba, RgbaImage};
use rise_canon::RiseWireKey;
use rise_canon::rise_key::{bot as bot_key, message as key, upload as upload_key};
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::RiseJSONValue;
use rise_engine::rise_upload::rise_chunked_upload::{RiseChunkedUploadClient, RiseChunkedUploadResult};
use serde_json::{Map, Value};

use crate::core::lib::byte_format::ByteFormat;
use crate::core::lib::rise_local_key::RiseLocalKey;
use crate::core::lib::rise_wire_key_camel::RiseWireKeyCamel;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::core::utilities::sai_cache_cleaner::SaiCacheCleanLayer;
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{SaiWsCachePreferredLayer, SaiWsDiskCachePolicy};
use crate::core::lib::sai::modules::sai_ws::sai_ws_response_cache::SaiWsResponseCache;
use crate::core::ui::clever_image_disk_cache::CleverImageDiskCache;
use crate::modules::chat::engine::core::rise_conversation_upload_support::RiseConversationUploadSupport;
use crate::modules::chat::engine::messages::rise_message_media_upload_pipeline::RiseMessageMediaUploadPipeline;
use crate::modules::chat::stores::message::message_actions::message_types::{
    GetMessageMessage, MessageDeliveryStatus, MessageMediaItem, MessageMediaKind,
};
use crate::modules::chat::stores::message::message_actions::message_wire_types::{
    MessagePayloadCodec, RiseStoredOutgoingMedia,
};
use crate::modules::chat::stores::message::message_services::message_services::MessageServicesStore;
use crate::modules::media_picker::api::media_compression_configuration::MediaCompressionConfiguration;
use crate::modules::media_picker::api::media_picker_configuration::MediaPickerConfiguration;
use crate::modules::media_picker::api::media_picker_result::{
    MediaPickerAudioPayload, MediaPickerEditedVideo, MediaPickerItem, MediaPickerItemKind, MediaPickerPhotoAssetSource,
};
use crate::modules::media_picker::editor::editor_state::{EditorImageState, EditorVideoState};
use crate::modules::media_picker::editor::editor_state_codable::EditorStateCodec;
use crate::modules::media_picker::export::outgoing_media_compressor::{
    OutgoingMediaCompressor, OutgoingMediaCompressorResult,
};
use crate::modules::media_picker::export::rise_outgoing_media_file::RiseOutgoingMediaFile;

/// removes the paths on drop, so it runs on a failed assertion too
struct RemoveOnDrop(Vec<PathBuf>);

impl Drop for RemoveOnDrop {
    fn drop(&mut self) {
        for path in &self.0 {
            if path.is_dir() {
                let _ = fs::remove_dir_all(path);
            } else {
                let _ = fs::remove_file(path);
            }
        }
    }
}

fn token() -> String {
    Uuid::new_v4().uuid_string()
}

fn temp_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(name)
}

fn file_url(path: &Path) -> Url {
    Url::from_file_path(path).expect("absolute temp path")
}

fn absolute(path: &Path) -> String {
    file_url(path).absolute_string().to_owned()
}

fn dict<const N: usize>(entries: [(&str, Value); N]) -> Map<String, Value> {
    entries.into_iter().map(|(name, value)| (name.to_owned(), value)).collect()
}

fn is_temp_key() -> String {
    RiseWireKey::camel(RiseLocalKey::IS_TEMP)
}

fn messages_root(row: Map<String, Value>) -> Value {
    Value::Object(dict([(key::MESSAGES, Value::Array(vec![Value::Object(row)]))]))
}

fn prepare_test(cx: &TestAppContext) {
    cx.executor().allow_parking();
    crate::install_feature_hooks();
}

fn store_update<R>(
    cx: &TestAppContext,
    update: impl FnOnce(&mut MessageServicesStore, &mut Context<MessageServicesStore>) -> R,
) -> R {
    cx.update(|cx| MessageServicesStore::shared(cx).update(cx, update))
}

fn messages(chat_id: &str, cx: &TestAppContext) -> Vec<GetMessageMessage> {
    store_update(cx, |store, cx| store.messages(chat_id, cx))
}

fn wire_temp_id(request_temp_id: &str, cx: &TestAppContext) -> Option<String> {
    cx.update(|cx| MessageServicesStore::shared(cx).read(cx).outgoing_wire_temp_id(request_temp_id))
}

fn extract_slot(
    done: &Map<String, Value>,
    local_file: &RiseOutgoingMediaFile,
    picker_item: Option<&MediaPickerItem>,
    cx: &TestAppContext,
) -> Map<String, Value> {
    cx.update(|cx| {
        MessageServicesStore::shared(cx).read(cx).extract_uploaded_media_slot(done, local_file, picker_item, None)
    })
}

fn finalize_audio(uploaded: &[Value], audio_index: i64, cover_index: Option<i64>, cx: &TestAppContext) -> Vec<Value> {
    cx.update(|cx| {
        MessageServicesStore::shared(cx).read(cx).finalize_audio_uploaded_slots(uploaded, audio_index, cover_index)
    })
}

fn install_root(root: Value, cache_key: &str, persist_disk: bool, cx: &TestAppContext) {
    cx.update(|cx| {
        SaiWsResponseCache::install_optimistic_root(
            root,
            cache_key,
            true,
            persist_disk,
            SaiWsDiskCachePolicy::PreserveExisting,
            false,
            cx,
        )
    })
    .expect("root installed");
}

fn remove_cache(cache_key: &str, cx: &TestAppContext) {
    cx.update(|cx| SaiWsResponseCache::remove(cache_key, SaiCacheCleanLayer::Everywhere, cx));
}

fn first_slot(root: &Map<String, Value>) -> Map<String, Value> {
    let message = MessagePayloadCodec::array_of_dictionaries(root.get(key::MESSAGES))
        .into_iter()
        .next()
        .expect("message row");
    MessagePayloadCodec::array_of_dictionaries(message.get(bot_key::MEDIA_ITEMS))
        .into_iter()
        .next()
        .expect("media slot")
}

fn chunked_result(asset_id: &str, file_url: &str, file_key: &str, mime_type: &str) -> RiseChunkedUploadResult {
    RiseChunkedUploadResult {
        asset_id: asset_id.to_owned(),
        file_url: file_url.to_owned(),
        file_key: file_key.to_owned(),
        access_url: None,
        thumbnail_url: None,
        file_id: None,
        mime_type: mime_type.to_owned(),
        file_size: None,
        width: None,
        height: None,
        duration: None,
        metadata: BTreeMap::new(),
    }
}

fn outgoing_file(url: PathBuf, filename: &str, content_type: &str, media_type: Option<&str>) -> RiseOutgoingMediaFile {
    let mut file = RiseOutgoingMediaFile::new(url, filename.to_owned(), content_type.to_owned());
    file.media_type = media_type.map(str::to_owned);
    file
}

fn compressed_result(
    url: Url,
    content_type: &str,
    byte_count: i64,
    pixel_width: Option<i64>,
    pixel_height: Option<i64>,
) -> OutgoingMediaCompressorResult {
    OutgoingMediaCompressorResult {
        url,
        content_type: content_type.to_owned(),
        byte_count,
        pixel_width,
        pixel_height,
    }
}

fn jpeg_data(image: &RgbImage, quality: u8) -> Vec<u8> {
    let mut data = Vec::new();
    JpegEncoder::new_with_quality(&mut data, quality)
        .encode(image.as_raw(), image.width(), image.height(), ExtendedColorType::Rgb8)
        .expect("jpeg encoded");
    data
}

#[gpui::test]
fn three_selected_images_create_one_ordered_optimistic_album(cx: &mut TestAppContext) {
    prepare_test(cx);
    let token = token();
    let chat_id = format!("optimistic-album-{token}");
    let request_temp_id = format!("request-album-{token}");
    let coordinator_temp_id = format!("temp-album-{token}");
    let cache_key = MessageServicesStore::messages_cache_key(&chat_id);
    let directory = temp_path(&format!("optimistic-album-{token}"));
    fs::create_dir_all(&directory).expect("directory created");
    let _cleanup = RemoveOnDrop(vec![directory.clone()]);

    let paths: Vec<PathBuf> = (0..3_usize)
        .map(|index| {
            let path = directory.join(format!("image-{index}.jpg"));
            fs::write(&path, vec![index as u8 + 1; 1_024 + index]).expect("file written");
            path
        })
        .collect();
    let items: Vec<MediaPickerItem> = paths
        .iter()
        .enumerate()
        .map(|(index, path)| {
            MediaPickerItem::new(
                format!("asset-{index}"),
                MediaPickerItemKind::Image,
                file_url(path),
                format!("image-{index}.jpg"),
                Some(format!("asset-{index}")),
                None,
                None,
                None,
                false,
            )
        })
        .collect();
    let local_files: Vec<RiseOutgoingMediaFile> = paths
        .iter()
        .zip(&items)
        .map(|(path, item)| outgoing_file(path.clone(), &item.suggested_file_name, "image/jpeg", None))
        .collect();

    let outbound = Value::Object(dict([
        (bot_key::CONTENT_TYPE, Value::from("image")),
        (bot_key::MEDIA_GROUP_ID, Value::from("album-1")),
    ]));
    let optimistic = store_update(cx, |store, cx| {
        store.create_optimistic_message_data(&outbound, &chat_id, &coordinator_temp_id, &local_files, &items, &[], cx)
    });
    let slots = MessagePayloadCodec::array_of_dictionaries(optimistic.get(bot_key::MEDIA_ITEMS));

    install_root(
        Value::Object(dict([(key::MESSAGES, Value::Array(Vec::new()))])),
        &cache_key,
        false,
        cx,
    );
    for _ in 0..2 {
        store_update(cx, |store, cx| {
            store.append_optimistic_from_coordinator_row(
                &optimistic,
                &request_temp_id,
                &chat_id,
                &items,
                &local_files,
                &[],
                cx,
            )
        });
    }

    assert_eq!(optimistic.get(bot_key::ID).and_then(Value::as_str), Some(coordinator_temp_id.as_str()));
    assert_eq!(optimistic.get(bot_key::HAS_ATTACHMENTS).and_then(Value::as_bool), Some(true));
    assert_eq!(slots.len(), 3);
    for (index, slot) in slots.iter().enumerate() {
        assert_eq!(slot.get(bot_key::ID).and_then(Value::as_str), Some(format!("temp-media-{index}").as_str()));
        assert_eq!(slot.get(RiseLocalKey::LOCAL_SLOT_INDEX).and_then(Value::as_i64), Some(index as i64));
        assert_eq!(slot.get(bot_key::FILE_URL).and_then(Value::as_str), Some(absolute(&paths[index]).as_str()));
        assert_eq!(slot.get(bot_key::FILE_NAME).and_then(Value::as_str), Some(format!("image-{index}.jpg").as_str()));
        assert_eq!(slot.get(bot_key::MEDIA_TYPE).and_then(Value::as_str), Some("image"));
        assert_eq!(slot.get(RiseLocalKey::IS_UPLOADING).and_then(Value::as_bool), Some(true));
    }

    let rows: Vec<GetMessageMessage> =
        messages(&chat_id, cx).into_iter().filter(|message| message.id == coordinator_temp_id).collect();
    let message = rows.first().expect("optimistic row");
    assert_eq!(rows.len(), 1);
    assert!(message.is_temp);
    assert!(message.delivery_status == MessageDeliveryStatus::Sending);
    assert_eq!(
        message.media_items.iter().map(|item| item.id.clone()).collect::<Vec<_>>(),
        vec![
            format!("{coordinator_temp_id}-0"),
            format!("{coordinator_temp_id}-1"),
            format!("{coordinator_temp_id}-2"),
        ]
    );
    assert_eq!(message.media_items.iter().map(|item| item.local_slot_index).collect::<Vec<_>>(), vec![0, 1, 2]);
    assert!(message.media_items.iter().all(|item| item.is_uploading));
    assert_eq!(wire_temp_id(&request_temp_id, cx), Some(coordinator_temp_id.clone()));

    store_update(cx, |store, _| store.clear_outgoing_wire_temp(&request_temp_id));
    remove_cache(&cache_key, cx);
}

#[gpui::test]
fn concurrent_optimistic_rows_keep_independent_wire_temp_ids(cx: &mut TestAppContext) {
    prepare_test(cx);
    let scope = format!("wire-temp-scope-{}", token());
    let first_request = format!("temp-request-1-{}", token());
    let second_request = format!("temp-request-2-{}", token());

    let first_row = dict([(bot_key::ID, Value::from("wire-first")), (bot_key::CONTENT_TYPE, Value::from("image"))]);
    let second_row = dict([(bot_key::ID, Value::from("wire-second")), (bot_key::CONTENT_TYPE, Value::from("image"))]);
    store_update(cx, |store, cx| {
        store.append_optimistic_from_coordinator_row(&first_row, &first_request, &scope, &[], &[], &[], cx)
    });
    store_update(cx, |store, cx| {
        store.append_optimistic_from_coordinator_row(&second_row, &second_request, &scope, &[], &[], &[], cx)
    });

    assert_eq!(wire_temp_id(&first_request, cx).as_deref(), Some("wire-first"));
    assert_eq!(wire_temp_id(&second_request, cx).as_deref(), Some("wire-second"));

    store_update(cx, |store, _| {
        store.clear_outgoing_wire_temp(&first_request);
        store.clear_outgoing_wire_temp(&second_request);
    });
    remove_cache(&MessageServicesStore::messages_cache_key(&scope), cx);
}

#[gpui::test]
fn audio_upload_keeps_only_audio_and_uses_uploaded_cover(cx: &mut TestAppContext) {
    prepare_test(cx);
    let cover = dict([
        (bot_key::MEDIA_TYPE, Value::from("image")),
        (bot_key::FILE_URL, Value::from("https://cdn.example/cover.jpg")),
    ]);
    let audio = dict([
        (bot_key::MEDIA_TYPE, Value::from("audio")),
        (bot_key::FILE_URL, Value::from("https://cdn.example/track.mp3")),
        (bot_key::FILE_NAME, Value::from("Artist \u{2014} Title.mp3")),
        (bot_key::DURATION, Value::from(184)),
    ]);

    let result = finalize_audio(&[Value::Object(cover), Value::Object(audio)], 1, Some(0), cx);
    let finalized = result.first().and_then(Value::as_object).expect("finalized slot");

    assert_eq!(result.len(), 1);
    assert_eq!(finalized.get(bot_key::MEDIA_TYPE).and_then(Value::as_str), Some("audio"));
    assert_eq!(finalized.get(bot_key::FILE_URL).and_then(Value::as_str), Some("https://cdn.example/track.mp3"));
    assert_eq!(finalized.get(bot_key::THUMBNAIL_URL).and_then(Value::as_str), Some("https://cdn.example/cover.jpg"));
    assert_eq!(finalized.get(bot_key::FILE_NAME).and_then(Value::as_str), Some("Artist \u{2014} Title.mp3"));
    assert_eq!(finalized.get(bot_key::DURATION).and_then(Value::as_i64), Some(184));
}

#[gpui::test]
fn outbound_audio_upload_does_not_expose_device_local_preview_url(cx: &mut TestAppContext) {
    prepare_test(cx);
    let local_path = temp_path(&format!("outbound-audio-{}.mp3", token()));
    fs::write(&local_path, vec![9_u8; 1_024]).expect("file written");
    let _cleanup = RemoveOnDrop(vec![local_path.clone()]);
    let local_url = file_url(&local_path);
    let filename = local_url.last_path_component();

    let local_file = outgoing_file(local_path.clone(), &filename, "audio/mpeg", Some("audio"));
    let picker_item = MediaPickerItem::new(
        token(),
        MediaPickerItemKind::File,
        local_url.clone(),
        filename.clone(),
        None,
        None,
        None,
        Some(MediaPickerAudioPayload {
            title: "Title".to_owned(),
            artist: "Artist".to_owned(),
            duration: 184.0,
            cover_url: None,
        }),
        false,
    );
    let done = dict([
        (bot_key::FILE_URL, Value::from("https://cdn.riseonly.net/public/chat/track.mp3")),
        (bot_key::MIME_TYPE, Value::from("audio/mpeg")),
        (bot_key::FILE_SIZE, Value::from(1_024)),
    ]);
    let ui_slot = extract_slot(&done, &local_file, Some(&picker_item), cx);
    let finalized = finalize_audio(&[Value::Object(ui_slot)], 0, None, cx);
    let finalized_slot = finalized.first().and_then(Value::as_object).expect("finalized slot");
    let outbound_slot = RiseConversationUploadSupport::sanitized_slot_for_outbound(finalized_slot);

    assert_eq!(
        finalized_slot.get(RiseLocalKey::LOCAL_PREVIEW_URL).and_then(Value::as_str),
        Some(local_url.absolute_string())
    );
    assert!(outbound_slot.get(RiseLocalKey::LOCAL_PREVIEW_URL).is_none());
    assert!(outbound_slot.get(&RiseWireKey::camel(RiseLocalKey::LOCAL_PREVIEW_URL)).is_none());
    assert_eq!(
        outbound_slot.get(bot_key::FILE_URL).and_then(Value::as_str),
        Some("https://cdn.riseonly.net/public/chat/track.mp3")
    );
}

#[gpui::test]
fn audio_upload_without_cover_preserves_audio_payload(cx: &mut TestAppContext) {
    prepare_test(cx);
    let audio = dict([
        (bot_key::MEDIA_TYPE, Value::from("audio")),
        (bot_key::FILE_URL, Value::from("https://cdn.example/track.mp3")),
    ]);

    let result = finalize_audio(&[Value::Object(audio)], 0, None, cx);
    let finalized = result.first().and_then(Value::as_object).expect("finalized slot");

    assert_eq!(result.len(), 1);
    assert_eq!(finalized.get(bot_key::FILE_URL).and_then(Value::as_str), Some("https://cdn.example/track.mp3"));
    assert!(finalized.get(bot_key::THUMBNAIL_URL).is_none());
}

#[gpui::test]
fn invalid_audio_index_does_not_corrupt_uploaded_slots(cx: &mut TestAppContext) {
    prepare_test(cx);
    let uploaded = vec![Value::Object(dict([(bot_key::MEDIA_TYPE, Value::from("document"))]))];

    let result = finalize_audio(&uploaded, 4, None, cx);

    assert_eq!(result.len(), uploaded.len());
}

#[gpui::test]
fn upload_progress_updates_memory_without_rewriting_disk(cx: &mut TestAppContext) {
    prepare_test(cx);
    let token = token();
    let chat_id = format!("upload-progress-{token}");
    let message_id = format!("temp-upload-progress-{token}");
    let cache_key = MessageServicesStore::messages_cache_key(&chat_id);
    let is_temp = is_temp_key();
    let root = messages_root(dict([
        (bot_key::ID, Value::from(message_id.clone())),
        (is_temp.as_str(), Value::from(true)),
        (
            bot_key::MEDIA_ITEMS,
            Value::Array(vec![Value::Object(dict([
                (bot_key::ID, Value::from("temp-media-0")),
                (bot_key::MEDIA_TYPE, Value::from("image")),
                (bot_key::FILE_URL, Value::from("file:///tmp/upload.jpg")),
                (RiseLocalKey::IS_UPLOADING, Value::from(true)),
                (RiseLocalKey::UPLOAD_PROGRESS, Value::from(0)),
            ]))]),
        ),
    ]));

    install_root(root, &cache_key, true, cx);
    let initial_rows = messages(&chat_id, cx);
    let initial_message = initial_rows.first().expect("initial row");
    assert_eq!(initial_message.media_items.first().map(|item| item.upload_progress), Some(0.0));

    store_update(cx, |store, cx| {
        store.patch_outgoing_media_upload_progress(
            &chat_id,
            &message_id,
            0,
            false,
            100.0,
            42.0,
            420,
            Some(1_000),
            cx,
        )
    });

    let memory_payload = cx.update(|cx| {
        SaiWsResponseCache::current_payload(
            &cache_key,
            SaiWsCachePreferredLayer::MemoryFirst,
            SaiWsDiskCachePolicy::PreserveExisting,
            cx,
        )
    });
    let memory_root = MessagePayloadCodec::dictionary(memory_payload.as_ref()).expect("memory root");
    let memory_slot = first_slot(&memory_root);
    assert_eq!(memory_slot.get(RiseLocalKey::UPLOAD_PROGRESS).and_then(Value::as_f64), Some(42.0));
    assert_eq!(memory_slot.get(RiseLocalKey::UPLOADED_BYTES).and_then(Value::as_i64), Some(420));
    assert_eq!(memory_slot.get(RiseLocalKey::TOTAL_BYTES).and_then(Value::as_i64), Some(1_000));
    let patched_rows = messages(&chat_id, cx);
    let patched_item = patched_rows.first().and_then(|message| message.media_items.first()).expect("patched item");
    assert_eq!(patched_item.upload_progress, 42.0);
    assert_eq!(patched_item.uploaded_bytes, 420);

    let disk_payload = cx.update(|cx| SaiWsResponseCache::disk_only(&cache_key, cx));
    let disk_root = MessagePayloadCodec::dictionary(disk_payload.as_ref()).expect("disk root");
    let disk_slot = first_slot(&disk_root);
    assert_eq!(disk_slot.get(RiseLocalKey::UPLOAD_PROGRESS).and_then(Value::as_f64), Some(0.0));
    assert!(disk_slot.get(RiseLocalKey::UPLOADED_BYTES).is_none());
    assert!(disk_slot.get(RiseLocalKey::TOTAL_BYTES).is_none());

    store_update(cx, |store, _| {
        store.upload_progress_throttle.remove(&format!("{message_id}#0"));
    });
    remove_cache(&cache_key, cx);
}

#[gpui::test]
fn uploaded_mp4_file_keeps_document_semantics_and_original_name(cx: &mut TestAppContext) {
    prepare_test(cx);
    let path = temp_path("original-video.mp4");
    fs::write(&path, vec![7_u8; 4_096]).expect("file written");
    let _cleanup = RemoveOnDrop(vec![path.clone()]);

    let local_file = outgoing_file(path.clone(), "original-video.mp4", "video/mp4", Some("document"));
    let done = dict([
        (bot_key::MEDIA_TYPE, Value::from("video")),
        (bot_key::FILE_URL, Value::from("https://cdn.riseonly.net/public/chat/server.mp4")),
        (bot_key::FILE_NAME, Value::from("server-generated-id")),
        (bot_key::FILE_SIZE, Value::from(0)),
    ]);
    let slot = extract_slot(&done, &local_file, None, cx);

    assert_eq!(slot.get(bot_key::MEDIA_TYPE).and_then(Value::as_str), Some("document"));
    assert_eq!(slot.get(bot_key::FILE_NAME).and_then(Value::as_str), Some("original-video.mp4"));
    assert_eq!(slot.get(bot_key::FILE_SIZE).and_then(Value::as_i64), Some(4_096));
}

#[tokio::test]
async fn document_upload_skips_media_compression() {
    let path = temp_path("uncompressed-file.mp4");
    let data = vec![3_u8; 8_192];
    fs::write(&path, &data).expect("file written");
    let _cleanup = RemoveOnDrop(vec![path.clone()]);

    let local_file = outgoing_file(path.clone(), "uncompressed-file.mp4", "video/mp4", Some("document"));
    let result =
        OutgoingMediaCompressor::compress_if_needed(local_file, None, MediaCompressionConfiguration::chat(), None, None)
            .await
            .expect("document passes through");

    assert_eq!(result.url, file_url(&path));
    assert_eq!(result.byte_count, data.len() as i64);
    assert_eq!(result.content_type, "video/mp4");
}

#[tokio::test]
async fn chat_image_compression_reports_the_exact_prepared_bitmap() {
    let max_side = MediaCompressionConfiguration::chat().profile.image_max_side as f64;
    let source_width = (max_side * 1.25).round();
    let source_height = (source_width / 1.5).round();
    let source = RgbaImage::from_pixel(source_width as u32, source_height as u32, Rgba([88, 86, 214, 255]));
    let mut source_data = Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(source).write_to(&mut source_data, ImageFormat::Png).expect("png encoded");
    let source_path = temp_path(&format!("chat-source-{}.png", token()));
    fs::write(&source_path, source_data.into_inner()).expect("file written");
    let mut cleanup = RemoveOnDrop(vec![source_path.clone()]);
    let filename = file_url(&source_path).last_path_component();

    let result = OutgoingMediaCompressor::compress_if_needed(
        outgoing_file(source_path.clone(), &filename, "image/png", None),
        None,
        MediaCompressionConfiguration::chat(),
        None,
        None,
    )
    .await
    .expect("image compressed");
    let prepared_path = result.url.to_file_path().expect("prepared file");
    cleanup.0.push(prepared_path.clone());

    assert_eq!(result.content_type, "image/jpeg");
    assert_eq!(result.url.path_extension(), "jpg");
    assert_eq!(result.pixel_width, Some(max_side as i64));
    let expected_height = (source_height * (max_side / source_width)).round() as i64;
    assert!((result.pixel_height.unwrap_or(0) - expected_height).abs() <= 1);
    assert!(result.byte_count > 0);
    let prepared_data = fs::read(&prepared_path).expect("prepared bytes");
    assert_eq!(result.byte_count, prepared_data.len() as i64);
}

#[tokio::test]
async fn high_entropy_chat_image_honors_the_byte_cap() {
    let profile = MediaCompressionConfiguration::chat().profile;
    let width = profile.image_max_side as u32;
    let height = width * 2 / 3;
    let mut state: u32 = 0x00C0_FFEE;
    let noise = RgbImage::from_fn(width, height, |_, _| {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        Rgb([state as u8, (state >> 8) as u8, (state >> 16) as u8])
    });
    let source_path = temp_path(&format!("chat-noise-{}.jpg", token()));
    fs::write(&source_path, jpeg_data(&noise, 100)).expect("file written");
    let mut cleanup = RemoveOnDrop(vec![source_path.clone()]);
    let filename = file_url(&source_path).last_path_component();

    let result = OutgoingMediaCompressor::compress_if_needed(
        outgoing_file(source_path.clone(), &filename, "image/jpeg", None),
        None,
        MediaCompressionConfiguration::chat(),
        None,
        None,
    )
    .await
    .expect("image compressed");
    let prepared_path = result.url.to_file_path().expect("prepared file");
    cleanup.0.push(prepared_path.clone());

    assert!(result.byte_count <= profile.image_max_bytes as i64);
    assert!(result.pixel_width.is_some());
    assert!(result.pixel_height.is_some());
    assert!(
        result.pixel_width.unwrap_or(i64::from(width)) < i64::from(width)
            || result.pixel_height.unwrap_or(i64::from(height)) < i64::from(height)
    );
    let prepared_data = fs::read(&prepared_path).expect("prepared bytes");
    assert_eq!(result.byte_count, prepared_data.len() as i64);
}

#[test]
fn document_preparation_preserves_the_exact_user_filename() {
    let original_path = temp_path("scan.custom-extension");
    let transformed_path = temp_path(&format!("ro-chat-img-{}.jpg", token()));
    let original = outgoing_file(original_path, "scan.custom-extension", "image/jpeg", Some("document"));

    let prepared = RiseConversationUploadSupport::prepared_local_file(
        &original,
        &compressed_result(file_url(&transformed_path), "image/jpeg", 128, Some(40), Some(30)),
    );

    assert_eq!(prepared.filename, "scan.custom-extension");
    assert_eq!(prepared.url, transformed_path);
    assert_eq!(prepared.media_type.as_deref(), Some("document"));
    assert_eq!(prepared.byte_count, Some(128));
}

#[test]
fn completion_uses_the_prepared_media_metadata() {
    let compressed = compressed_result(
        file_url(Path::new("/tmp/prepared.jpg")),
        "image/jpeg",
        12_345,
        Some(1_200),
        Some(800),
    );
    let mut result = chunked_result(
        "asset",
        "https://cdn.riseonly.net/public/chat/prepared.jpg",
        "public/chat/prepared.jpg",
        "image/jpeg",
    );
    result.metadata = BTreeMap::from([
        (upload_key::WIDTH.to_owned(), RiseJSONValue::Null),
        (upload_key::HEIGHT.to_owned(), RiseJSONValue::Integer(0)),
        (upload_key::FILE_SIZE.to_owned(), RiseJSONValue::String("0".to_owned())),
    ]);

    let completion = RiseConversationUploadSupport::completion_dictionary(&result, &compressed);

    assert_eq!(completion.get(upload_key::WIDTH).and_then(Value::as_i64), Some(1_200));
    assert_eq!(completion.get(upload_key::HEIGHT).and_then(Value::as_i64), Some(800));
    assert_eq!(completion.get(upload_key::FILE_SIZE).and_then(Value::as_i64), Some(12_345));
    assert_eq!(completion.get(upload_key::MIME_TYPE).and_then(Value::as_str), Some("image/jpeg"));
}

#[tokio::test]
async fn prepared_image_seeds_the_exact_bytes_under_the_normalized_remote_key() {
    let image = RgbImage::from_pixel(32, 24, Rgb([255, 45, 85]));
    let data = jpeg_data(&image, 80);
    let local_path = temp_path(&format!("ro-chat-img-{}.jpg", token()));
    fs::write(&local_path, &data).expect("file written");
    let _cleanup = RemoveOnDrop(vec![local_path.clone()]);
    let remote_url = format!("https://cdn.riseonly.net/public/chat/{}.jpg", token());
    let user_id = SaiAccountCacheScope::active_user_id();

    RiseConversationUploadSupport::seed_prepared_image_cache_if_possible(
        &compressed_result(file_url(&local_path), "image/jpeg", data.len() as i64, Some(32), Some(24)),
        &remote_url,
        user_id.clone(),
        SaiAccountCacheScope::active_epoch(),
    )
    .await;

    assert_eq!(CleverImageDiskCache::image_data(&remote_url, user_id.as_deref(), None), Some(data));

    CleverImageDiskCache::remove_cache_key(&remote_url, user_id.as_deref(), None);
}

#[test]
fn evicted_generated_preview_falls_back_to_the_remote_image() {
    let missing_path = temp_path(&format!("ro-chat-img-{}.jpg", token()));
    let mut item = MessageMediaItem::new(
        "remote-fallback".to_owned(),
        MessageMediaKind::Image,
        "https://cdn.riseonly.net/public/chat/remote.jpg".to_owned(),
    );
    item.width = 1_200.0;
    item.height = 800.0;
    item.local_preview_url = Some(absolute(&missing_path));

    assert_eq!(item.resolved_local_preview_url(), None);
    assert_eq!(item.display_source(), item.url);
}

#[gpui::test]
fn prepared_image_immediately_replaces_the_optimistic_picker_preview(cx: &mut TestAppContext) {
    prepare_test(cx);
    let token = token();
    let chat_id = format!("prepared-image-{token}");
    let message_id = format!("temp-prepared-image-{token}");
    let cache_key = MessageServicesStore::messages_cache_key(&chat_id);
    let original_path = temp_path(&format!("original-{token}.png"));
    let prepared_path = temp_path(&format!("prepared-{token}.jpg"));
    fs::write(&original_path, [0x89, 0x50, 0x4E, 0x47]).expect("file written");
    fs::write(&prepared_path, [0xFF, 0xD8, 0xFF, 0xD9]).expect("file written");
    let _cleanup = RemoveOnDrop(vec![original_path.clone(), prepared_path.clone()]);
    let original_url = absolute(&original_path);
    let prepared_url = absolute(&prepared_path);
    let is_temp = is_temp_key();

    install_root(
        messages_root(dict([
            (bot_key::ID, Value::from(message_id.clone())),
            (bot_key::CHAT_ID, Value::from(chat_id.clone())),
            (bot_key::CONTENT_TYPE, Value::from("image")),
            (is_temp.as_str(), Value::from(true)),
            (
                bot_key::MEDIA_ITEMS,
                Value::Array(vec![Value::Object(dict([
                    (bot_key::ID, Value::from("temp-media-0")),
                    (bot_key::MEDIA_TYPE, Value::from("image")),
                    (bot_key::FILE_URL, Value::from(original_url.clone())),
                    (bot_key::THUMBNAIL_URL, Value::from(original_url.clone())),
                    (RiseLocalKey::LOCAL_PREVIEW_URL, Value::from(original_url)),
                    (bot_key::WIDTH, Value::from(2_400)),
                    (bot_key::HEIGHT, Value::from(1_600)),
                    (RiseLocalKey::IS_COMPRESSING, Value::from(true)),
                ]))]),
            ),
        ])),
        &cache_key,
        false,
        cx,
    );
    let _ = messages(&chat_id, cx);

    let result = compressed_result(file_url(&prepared_path), "image/jpeg", 4, Some(1_200), Some(800));
    store_update(cx, |store, cx| {
        store.apply_outgoing_media_compression_result(&chat_id, &message_id, 0, &result, cx)
    });

    let rows = messages(&chat_id, cx);
    let item = rows.first().and_then(|message| message.media_items.first()).expect("media item");
    assert_eq!(item.url, prepared_url);
    assert_eq!(item.thumbnail_url.as_deref(), Some(prepared_url.as_str()));
    assert_eq!(item.local_preview_url.as_deref(), Some(prepared_url.as_str()));
    assert_eq!(item.width, 1_200.0);
    assert_eq!(item.height, 800.0);
    assert_eq!(item.mime_type.as_deref(), Some("image/jpeg"));
    assert_eq!(item.file_size, Some(4));
    assert!(!item.is_compressing);
    assert!(item.is_uploading);

    remove_cache(&cache_key, cx);
}

#[gpui::test]
fn upload_completion_prefers_authoritative_image_geometry(cx: &mut TestAppContext) {
    prepare_test(cx);
    let token = token();
    let chat_id = format!("uploaded-geometry-{token}");
    let message_id = format!("temp-uploaded-geometry-{token}");
    let cache_key = MessageServicesStore::messages_cache_key(&chat_id);
    let preview_path = temp_path(&format!("prepared-geometry-{token}.jpg"));
    fs::write(&preview_path, [0xFF, 0xD8, 0xFF, 0xD9]).expect("file written");
    let _cleanup = RemoveOnDrop(vec![preview_path.clone()]);
    let preview_url = absolute(&preview_path);
    let remote_url = "https://cdn.riseonly.net/private/chats/prepared.jpg";
    let is_temp = is_temp_key();

    install_root(
        messages_root(dict([
            (bot_key::ID, Value::from(message_id.clone())),
            (bot_key::CHAT_ID, Value::from(chat_id.clone())),
            (bot_key::CONTENT_TYPE, Value::from("image")),
            (is_temp.as_str(), Value::from(true)),
            (
                bot_key::MEDIA_ITEMS,
                Value::Array(vec![Value::Object(dict([
                    (bot_key::ID, Value::from("temp-media-0")),
                    (bot_key::MEDIA_TYPE, Value::from("image")),
                    (bot_key::FILE_URL, Value::from(preview_url.clone())),
                    (RiseLocalKey::LOCAL_PREVIEW_URL, Value::from(preview_url.clone())),
                    (bot_key::WIDTH, Value::from(1_920)),
                    (bot_key::HEIGHT, Value::from(1_080)),
                    (RiseLocalKey::IS_UPLOADING, Value::from(true)),
                ]))]),
            ),
        ])),
        &cache_key,
        false,
        cx,
    );
    let _ = messages(&chat_id, cx);

    let slot = dict([
        (bot_key::MEDIA_TYPE, Value::from("image")),
        (bot_key::FILE_URL, Value::from(remote_url)),
        (bot_key::FILE_NAME, Value::from("prepared.jpg")),
        (bot_key::MIME_TYPE, Value::from("image/jpeg")),
        (bot_key::WIDTH, Value::from(1_200)),
        (bot_key::HEIGHT, Value::from(800)),
    ]);
    let local_file = outgoing_file(preview_path.clone(), "prepared.jpg", "image/jpeg", None);
    store_update(cx, |store, cx| store.apply_uploaded_media_slot(&chat_id, &message_id, 0, &slot, &local_file, cx));

    let rows = messages(&chat_id, cx);
    let item = rows.first().and_then(|message| message.media_items.first()).expect("media item");
    assert_eq!(item.width, 1_200.0);
    assert_eq!(item.height, 800.0);
    assert_eq!(item.url, remote_url);
    assert_eq!(item.local_preview_url.as_deref(), Some(preview_url.as_str()));
    assert_eq!(item.display_source(), item.url);
    assert!(!item.is_uploading);

    fs::remove_file(&preview_path).expect("preview removed");
    assert_eq!(item.resolved_local_preview_url(), None);
    assert_eq!(item.display_source(), remote_url);

    remove_cache(&cache_key, cx);
}

#[test]
fn slab_progress_includes_bytes_inside_current_server_slab() {
    let progress = RiseChunkedUploadClient::overall_progress(5 * 1_024 * 1_024, 512 * 1_024, 10 * 1_024 * 1_024);

    assert!((progress - 0.55).abs() < 0.000_001);
}

#[test]
fn transferred_bytes_use_the_totals_automatic_display_unit() {
    let three_mib: i64 = 3 * 1_024 * 1_024;
    let four_mib: i64 = 4 * 1_024 * 1_024;
    assert_eq!(ByteFormat::telegram_transferred(three_mib, four_mib), ByteFormat::telegram(three_mib));

    let two_gib: i64 = 2 * 1_024 * 1_024 * 1_024;
    let three_gib: i64 = 3 * 1_024 * 1_024 * 1_024;
    assert_eq!(ByteFormat::telegram_transferred(two_gib, three_gib), ByteFormat::telegram(two_gib));
}

#[test]
fn deferred_photo_image_keeps_stable_source_and_edit_recipe_without_bytes() {
    let asset_identifier = format!("{}/L0/001", token());
    let source_pixel_size = size(4_000.0_f32, 3_000.0_f32);
    let mut edit_state = EditorImageState::empty(Some(asset_identifier.clone()), source_pixel_size);
    edit_state.rotation_steps = 1;
    edit_state.crop_rect_relative = Some(bounds(point(0.2, 0.1), size(0.5, 0.75)));
    edit_state.caption = "caption".to_owned();
    edit_state.synchronize_edit_flag();

    let item =
        MediaPickerItem::deferred_photo_library_image(&asset_identifier, source_pixel_size, edit_state.clone(), None)
            .expect("asset url");
    let payload = item.image_payload.as_ref().expect("image payload");
    let encoded_state = payload.editor_state_data.as_ref().expect("encoded state");

    assert_eq!(item.id, format!("ph-image:{asset_identifier}"));
    assert_eq!(item.url.absolute_string(), format!("ph://{asset_identifier}"));
    assert!(!item.url.is_file_url());
    assert!(item.suggested_file_name.ends_with(".jpg"));
    assert_eq!(item.deferred_photo_library_image_asset_identifier(), Some(asset_identifier.clone()));
    assert!(payload.data.is_empty());
    assert!(payload.was_edited);
    assert_eq!(payload.caption, "caption");
    assert_eq!(payload.pixel_size, size(960.0, 1_920.0));
    assert!(payload.original_image_url.is_none());
    assert!(payload.is_processing_deferred);
    assert_eq!(payload.deferred_edit_state.as_ref(), Some(&edit_state));
    assert_eq!(EditorStateCodec::decode_image(encoded_state), Some(edit_state));
}

#[test]
fn every_chat_gallery_picker_defers_photo_library_processing_to_rise() {
    assert!(MediaPickerConfiguration::chat_attachments().defers_video_processing_to_consumer);
    assert!(MediaPickerConfiguration::chat_video_note().defers_video_processing_to_consumer);
    assert!(!MediaPickerConfiguration::chat_attachments().shows_export_progress_overlay);
    assert!(!MediaPickerConfiguration::chat_video_note().shows_export_progress_overlay);
}

#[test]
fn deferred_photo_video_uses_stable_source_without_materialized_file() {
    let asset_identifier = "E72D529C-4F8B-44CE-85D2-4EE25FBD64B8/L0/001";
    let mut edit_state = EditorVideoState::empty(Some(asset_identifier.to_owned()), 12.0);
    edit_state.start_seconds = 2.0;
    edit_state.end_seconds = 9.0;
    edit_state.is_muted = true;
    edit_state.caption = "caption".to_owned();

    let item = MediaPickerItem::deferred_photo_library_video(
        asset_identifier,
        size(1_920.0, 1_080.0),
        edit_state,
        None,
    )
    .expect("asset url");
    let payload = item.video_payload.as_ref().expect("video payload");

    assert_eq!(item.id, format!("ph-video:{asset_identifier}"));
    assert_eq!(item.url.absolute_string(), format!("ph://{asset_identifier}"));
    assert_eq!(payload.url, item.url);
    assert_eq!(payload.poster_url.as_ref(), Some(&item.url));
    assert_eq!(payload.start_seconds, 2.0);
    assert_eq!(payload.end_seconds, 9.0);
    assert_eq!(payload.duration, 7.0);
    assert!(payload.is_muted);
    assert_eq!(payload.caption, "caption");
    assert_eq!(payload.pixel_size, size(1_920.0, 1_080.0));
    assert!(payload.is_processing_deferred);
    assert_eq!(item.deferred_photo_library_video_asset_identifier().as_deref(), Some(asset_identifier));
    assert!(item.suggested_file_name.ends_with(".mp4"));
}

#[test]
fn rise_upload_result_builds_exact_create_message_wire_keys() {
    let local_path = PathBuf::from("/tmp/photo.jpg");
    let compressed = compressed_result(file_url(&local_path), "image/jpeg", 12_345, Some(1_200), Some(800));
    let mut result = chunked_result(
        "asset-1",
        "https://cdn.riseonly.net/public/chat/photo.jpg",
        "public/chat/file-1.jpg",
        "image/jpeg",
    );
    result.file_size = Some(12_345);
    result.width = Some(1_200);
    result.height = Some(800);

    let body = RiseMessageMediaUploadPipeline::media_body(
        &result,
        &outgoing_file(local_path, "photo.jpg", "image/jpeg", None),
        &compressed,
        None,
        None,
    );
    let encoded = serde_json::to_value(&body).expect("body encoded");
    let root = encoded.as_object().expect("body object");

    assert_eq!(root.get(bot_key::MEDIA_TYPE).and_then(Value::as_str), Some("image"));
    assert_eq!(root.get(bot_key::FILE_ID).and_then(Value::as_str), Some("file-1"));
    assert_eq!(
        root.get(bot_key::FILE_URL).and_then(Value::as_str),
        Some("https://cdn.riseonly.net/public/chat/photo.jpg")
    );
    assert_eq!(root.get(bot_key::MIME_TYPE).and_then(Value::as_str), Some("image/jpeg"));
    assert!(root.get(&RiseWireKey::camel(bot_key::FILE_URL)).is_none());
}

#[test]
fn video_upload_keeps_local_metadata_when_completion_omits_it() {
    let local_path = PathBuf::from("/tmp/video.mp4");
    let local_url = file_url(&local_path);
    let compressed = compressed_result(local_url.clone(), "video/mp4", 4_096, None, None);
    let mut video = MediaPickerEditedVideo::new(local_url.clone(), 0.0, 7.6, 7.6, false, String::new());
    video.pixel_size = size(1_920.0, 1_080.0);
    let picker_item = MediaPickerItem::new(
        "video-1".to_owned(),
        MediaPickerItemKind::Video,
        local_url,
        "video.mp4".to_owned(),
        None,
        None,
        Some(video),
        None,
        false,
    );

    let body = RiseMessageMediaUploadPipeline::media_body(
        &chunked_result(
            "asset-video",
            "https://cdn.riseonly.net/public/chat/video.mp4",
            "public/chat/video-1.mp4",
            "video/mp4",
        ),
        &outgoing_file(local_path, "video.mp4", "video/mp4", None),
        &compressed,
        Some(&picker_item),
        None,
    );

    assert_eq!(body.width, Some(1_920));
    assert_eq!(body.height, Some(1_080));
    assert_eq!(body.duration, Some(8));
}

#[test]
fn visual_media_uses_only_exact_transport_byte_progress() {
    let photo = outgoing_file(PathBuf::from("/tmp/photo.jpg"), "photo.jpg", "image/jpeg", None);
    let video = outgoing_file(PathBuf::from("/tmp/video.mp4"), "video.mp4", "video/mp4", None);
    let image_document = outgoing_file(PathBuf::from("/tmp/source.png"), "source.png", "image/png", Some("document"));
    let video_note = outgoing_file(PathBuf::from("/tmp/note.mp4"), "note.mp4", "video/mp4", Some("video_note"));

    assert!(RiseMessageMediaUploadPipeline::uses_transport_byte_progress_exclusively(&photo));
    assert!(RiseMessageMediaUploadPipeline::uses_transport_byte_progress_exclusively(&video));
    assert!(!RiseMessageMediaUploadPipeline::uses_transport_byte_progress_exclusively(&image_document));
    assert!(!RiseMessageMediaUploadPipeline::uses_transport_byte_progress_exclusively(&video_note));
}

#[test]
fn final_outbound_media_drops_slot_cancelled_after_its_upload_finished() {
    let uploaded_by_index: HashMap<i64, RiseStoredOutgoingMedia> = (0..3)
        .map(|index| {
            let mut media = RiseStoredOutgoingMedia::new("image".to_owned());
            media.file_id = Some(format!("file-{index}"));
            media.file_url = Some(format!("https://cdn.riseonly.net/file-{index}.jpg"));
            (index, media)
        })
        .collect();

    let finalized =
        RiseMessageMediaUploadPipeline::finalized_media_bodies(uploaded_by_index, 3, &[], &HashSet::from([1]))
            .expect("bodies finalized");

    assert_eq!(
        finalized.iter().filter_map(|media| media.file_id.clone()).collect::<Vec<_>>(),
        vec!["file-0".to_owned(), "file-2".to_owned()]
    );
}

#[test]
fn upload_byte_state_clamps_progress_and_calculates_remaining_bytes() {
    let mut item =
        MessageMediaItem::new("document".to_owned(), MessageMediaKind::Document, "/tmp/file.bin".to_owned());
    item.file_size = Some(1_000);
    item.is_uploading = true;
    item.uploaded_bytes = 400;

    assert_eq!(item.effective_total_bytes(), 1_000);
    assert_eq!(item.clamped_uploaded_bytes(), 400);
    assert_eq!(item.remaining_upload_bytes(), 600);
    assert!((item.normalized_transfer_progress() - 0.4).abs() < 0.000_001);
}

#[test]
fn picked_file_asset_keeps_its_own_name_not_a_path_segment() {
    assert_eq!(MediaPickerPhotoAssetSource::suggested_image_file_name("/Users/someone/Pictures/al1.png"), "al1.jpg");
    assert_eq!(MediaPickerPhotoAssetSource::suggested_video_file_name("/Users/someone/Movies/clip.mov"), "clip.mp4");
    assert_eq!(MediaPickerPhotoAssetSource::suggested_image_file_name("C:\\Users\\someone\\al2.jpeg"), "al2.jpg");
    assert_eq!(
        MediaPickerPhotoAssetSource::suggested_image_file_name("1A2B-3C/L0/001"),
        "image-1A2B-3C.jpg"
    );
}
