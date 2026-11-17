use std::collections::HashSet;

use gpui::TestAppContext;
use rise_canon::rise_key::upload as upload_key;
use rise_canon::rise_view::message as view;
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::rise_upload::rise_url_session::URLErrorCode;
use rise_platform::file_manager::FileManager;
use rise_platform::paths::temporary_directory;
use rise_platform::system_share::ShareItem;
use serde_json::{Map, Value};

use crate::core::lib::any_dictionary_bridge::AnyDictionaryBridge;
use crate::core::lib::sai::modules::sai_ws::sai_ws_chunked_file_upload::SaiWsChunkedFileUpload;
use crate::core::lib::sai::modules::sai_ws::sai_ws_disk_cache::SaiWsDiskCache;
use crate::core::ui::formatted_text_ui::FormattedText;
use crate::core::widgets::sheets::message_external_share_materializer::MessageExternalShareMaterializer;
use crate::core::widgets::sheets::share_sheet::{
    SharePayload, ShareSheetDestination, ShareSheetForwardBatchPlanner, ShareSheetTextBatchPlanner,
};
use crate::modules::chat::stores::message::message_actions::message_types::{
    GetMessageMessage, MessageContentType, MessageMediaItem, MessageMediaKind,
};
use crate::modules::chat::stores::message::message_actions::message_wire_types::ForwardMessagesGatewayDTO;
use crate::modules::share_extension::send_handoff::share_send_handoff_models::RiseStoredSharePayloadReply;
use crate::modules::share_extension::services::rise_share_app_bridge::RiseShareAppBridge;
use crate::modules::share_extension::shared::rise_share_container::{RiseShareContainer, RiseShareContainerError};
use crate::modules::share_extension::shared::rise_share_models::{
    RiseShareAccount, RiseShareIntentDestinationID, RiseShareSection, RiseShareSnapshot, RiseShareStoryItem,
    RiseShareStoryMediaKind, RiseShareStoryPublishCheckpoint, RiseShareStorySession, RiseShareSuggestionEligibility,
    RiseShareTarget, RiseShareTargetKind, RiseShareTopic, RiseShareTopicGrouping,
};
use crate::modules::share_extension::stores::share_targets::share_targets_services::ShareTargetsServicesStore;
use crate::modules::share_extension::stores::share_targets::share_targets_wire_types::{
    GetShareTargetsGatewayDTO, GetShareTopicsGatewayDTO,
};

const MIB: i64 = 1_024 * 1_024;

fn url(string: &str) -> Url {
    let Some(url) = Url::from_string(string) else {
        panic!("test URL {string} does not parse");
    };
    url
}

fn decode_fixture<T: serde::de::DeserializeOwned>(json: &str) -> T {
    match serde_json::from_str::<T>(json) {
        Ok(value) => value,
        Err(error) => panic!("fixture does not decode: {error}"),
    }
}

fn target(id: &str, title: &str, kind: RiseShareTargetKind, can_send: bool, last_activity_ms: i64) -> RiseShareTarget {
    RiseShareTarget::new(
        id.to_owned(),
        title.to_owned(),
        None,
        None,
        kind,
        false,
        can_send,
        last_activity_ms,
    )
}

fn topic(id: &str, title: &str, section_id: &str, position: i64, is_general: bool) -> RiseShareTopic {
    RiseShareTopic {
        id: id.to_owned(),
        chat_id: "chat-42".to_owned(),
        title: title.to_owned(),
        icon: String::new(),
        section_id: section_id.to_owned(),
        position,
        is_general,
        can_send: true,
    }
}

fn section(id: &str, name: &str, position: i64, created_at_ms: i64) -> RiseShareSection {
    RiseShareSection {
        id: id.to_owned(),
        chat_id: "chat-42".to_owned(),
        name: name.to_owned(),
        position,
        created_at_ms,
    }
}

fn story_item(id: &str, kind: RiseShareStoryMediaKind, relative_path: &str, content_type: &str) -> RiseShareStoryItem {
    RiseShareStoryItem {
        id: id.to_owned(),
        kind,
        relative_path: relative_path.to_owned(),
        suggested_filename: relative_path.to_owned(),
        content_type: content_type.to_owned(),
    }
}

fn destinations(count: i64) -> Vec<ShareSheetDestination> {
    (0..count)
        .map(|index| ShareSheetDestination::new(&format!("chat-{index}"), None))
        .collect()
}

fn initiate_data(entries: &[(&str, i64)]) -> Map<String, Value> {
    entries
        .iter()
        .map(|(key, value)| ((*key).to_owned(), Value::from(*value)))
        .collect()
}

#[gpui::test]
fn async_disk_checkpoints_preserve_the_latest_queued_empty_snapshot(cx: &mut TestAppContext) {
    let cache_key = format!("test-music-pending-checkpoint-{}", Uuid::new_v4().uuid_string());

    let pending: Value = decode_fixture(r#"{"playlist":{"tracks":[{"id":"pending"}]}}"#);
    let empty: Value = decode_fixture(r#"{"playlist":{"tracks":[]}}"#);
    cx.update(|cx| {
        SaiWsDiskCache::save_asynchronously(cache_key.clone(), pending, |_, _| {}, cx);
        SaiWsDiskCache::save_asynchronously(cache_key.clone(), empty, |_, _| {}, cx);
    });
    SaiWsDiskCache::wait_for_asynchronous_writes();

    let loaded = SaiWsDiskCache::load(&cache_key);
    SaiWsDiskCache::remove(&cache_key);
    cx.run_until_parked();

    let Some(root) = AnyDictionaryBridge::dictionary(loaded.as_ref()) else {
        panic!("the checkpoint was not persisted");
    };
    let Some(playlist) = AnyDictionaryBridge::dictionary(root.get("playlist")) else {
        panic!("the checkpoint lost its playlist");
    };
    let Some(Value::Array(tracks)) = playlist.get("tracks") else {
        panic!("the checkpoint lost its tracks");
    };
    assert!(tracks.is_empty());
}

#[test]
fn chunked_upload_uses_server_preference_with_safe_bounds() {
    assert_eq!(SaiWsChunkedFileUpload::resolved_slab_size(None), 5 * MIB);
    assert_eq!(
        SaiWsChunkedFileUpload::resolved_slab_size(Some(&Value::from(2 * MIB))),
        2 * MIB
    );
    assert_eq!(
        SaiWsChunkedFileUpload::resolved_slab_size(Some(&Value::from("1048576"))),
        MIB
    );
    assert_eq!(SaiWsChunkedFileUpload::resolved_slab_size(Some(&Value::from(1))), 64 * 1_024);
    assert_eq!(
        SaiWsChunkedFileUpload::resolved_slab_size(Some(&Value::from(64 * MIB))),
        8 * MIB
    );
    assert_eq!(
        SaiWsChunkedFileUpload::resolved_slab_size_initiate_data(&initiate_data(&[
            (upload_key::SLAB_SIZE, 512 * 1_024),
            (upload_key::CHUNK_SIZE, 5 * MIB),
        ])),
        512 * 1_024
    );
    assert_eq!(
        SaiWsChunkedFileUpload::resolved_slab_size_initiate_data(&initiate_data(&[(
            upload_key::CHUNK_SIZE,
            2 * MIB
        )])),
        2 * MIB
    );
}

#[test]
fn chunked_upload_retries_only_replay_safe_offset_aware_slabs() {
    assert!(SaiWsChunkedFileUpload::supports_offset_aware_slabs(&initiate_data(&[(
        upload_key::SLAB_SIZE,
        512 * 1_024
    )])));
    assert!(!SaiWsChunkedFileUpload::supports_offset_aware_slabs(&initiate_data(&[(
        upload_key::CHUNK_SIZE,
        5 * MIB
    )])));
    assert!(SaiWsChunkedFileUpload::should_retry_slab(true, 0, Some(503), None));
    assert!(SaiWsChunkedFileUpload::should_retry_slab(
        true,
        1,
        None,
        Some(URLErrorCode::NetworkConnectionLost)
    ));
    assert!(!SaiWsChunkedFileUpload::should_retry_slab(false, 0, Some(503), None));
    assert!(!SaiWsChunkedFileUpload::should_retry_slab(true, 2, Some(503), None));
    assert!(!SaiWsChunkedFileUpload::should_retry_slab(true, 0, Some(409), None));
}

#[test]
fn share_suggestion_cleanup_removes_only_targets_missing_from_fresh_writable_set() {
    let previous: HashSet<String> = ["chat-1", "chat-2", "chat-3"].map(str::to_owned).into();
    let fresh: HashSet<String> = ["chat-2", "chat-4"].map(str::to_owned).into();

    let removed = RiseShareAppBridge::removed_writable_target_ids(&previous, &fresh);

    assert_eq!(removed, vec!["chat-1".to_owned(), "chat-3".to_owned()]);
}

#[test]
fn share_targets_decode_gateway_snake_case_contract() {
    let response: GetShareTargetsGatewayDTO = decode_fixture(
        r#"{
          "success": true,
          "targets": [{
            "chat_id": "chat-42",
            "title": "Core Team",
            "avatar_url": "https://riseonly.net/avatar.jpg",
            "color_key": "peer-7",
            "type": "group",
            "topics_enabled": true,
            "can_send": true,
            "last_activity": 1783992000123
          }],
          "next_offset": 100,
          "has_more": true
        }"#,
    );

    assert_eq!(response.next_offset, 100);
    assert!(response.has_more);
    let Some(target) = response.targets.first().and_then(ShareTargetsServicesStore::map_target) else {
        panic!("the writable target was dropped");
    };
    assert_eq!(target.id, "chat-42");
    assert_eq!(target.title, "Core Team");
    assert_eq!(target.avatar_color_key.as_deref(), Some("peer-7"));
    assert!(target.topics_enabled);
    assert!(target.can_send);
    assert_eq!(target.last_activity_ms, 1_783_992_000_123);
}

#[test]
fn share_topics_decode_gateway_snake_case_contract() {
    let response: GetShareTopicsGatewayDTO = decode_fixture(
        r#"{
          "success": true,
          "topics": [{
            "topic_id": "",
            "chat_id": "chat-42",
            "title": "General",
            "icon": "📣",
            "section_id": "main",
            "position": 0,
            "is_general": true,
            "can_send": true
          }, {
            "topic_id": "topic-7",
            "chat_id": "chat-42",
            "title": "iOS",
            "icon": "🍎",
            "section_id": "main",
            "position": 1,
            "is_general": false,
            "can_send": true
          }],
          "sections": [{
            "section_id": "main",
            "chat_id": "chat-42",
            "name": "Development",
            "position": 2,
            "created_at": 1783992000123
          }]
        }"#,
    );

    let (Some(general), Some(ios)) = (response.topics.first(), response.topics.last()) else {
        panic!("the topics were not decoded");
    };
    assert!(general.topic_id.is_empty());
    assert_eq!(general.chat_id, "chat-42");
    assert!(general.is_general);
    assert!(general.can_send);
    assert_eq!(ios.topic_id, "topic-7");
    assert_eq!(ios.icon, "🍎");
    let Some(section) = response.sections.first() else {
        panic!("the section was not decoded");
    };
    assert_eq!(section.section_id, "main");
    assert_eq!(section.chat_id, "chat-42");
    assert_eq!(section.name, "Development");
    assert_eq!(section.position, 2);
    assert_eq!(section.created_at, 1_783_992_000_123);
}

#[test]
fn share_topic_grouping_is_deterministic_and_keeps_orphans_visible() {
    let sections = vec![
        section("section-b", "Beta", 1, 20),
        section("section-empty", "Empty", 0, 10),
        section("section-a", "Alpha", 1, 30),
    ];
    let topics = vec![
        topic("a-2", "A2", "section-a", 3, false),
        topic("loose", "Loose", "", 2, false),
        topic("b-1", "B1", "section-b", 0, false),
        topic("", "General", "section-b", 9, true),
        topic("orphan", "Orphan", "missing", 0, false),
        topic("a-1", "A1", "section-a", 3, false),
    ];

    let groups = RiseShareTopicGrouping::groups(
        &sections,
        topics,
        |topic: &RiseShareTopic| topic.id.clone(),
        |topic: &RiseShareTopic| topic.section_id.clone(),
        |topic: &RiseShareTopic| topic.position,
        |topic: &RiseShareTopic| topic.is_general,
    );

    let section_ids: Vec<Option<&str>> = groups
        .iter()
        .map(|group| group.section.as_ref().map(|section| section.id.as_str()))
        .collect();
    assert_eq!(section_ids, vec![None, Some("section-a"), Some("section-b")]);
    let topic_ids: Vec<Vec<&str>> = groups
        .iter()
        .map(|group| group.topics.iter().map(|topic| topic.id.as_str()).collect())
        .collect();
    assert_eq!(topic_ids[0], vec!["", "orphan", "loose"]);
    assert_eq!(topic_ids[1], vec!["a-1", "a-2"]);
    assert_eq!(topic_ids[2], vec!["b-1"]);
}

#[test]
fn share_picker_cache_keys_isolate_search_and_permission_flags() {
    let baseline = ShareTargetsServicesStore::targets_cache_key("user-1", None, false, false, false);
    let search = ShareTargetsServicesStore::targets_cache_key("user-1", Some(" Core "), false, false, false);
    let link = ShareTargetsServicesStore::targets_cache_key("user-1", None, false, true, false);

    assert_ne!(baseline, search);
    assert_ne!(baseline, link);
    assert_eq!(
        search,
        ShareTargetsServicesStore::targets_cache_key("user-1", Some("core"), false, false, false)
    );
}

#[test]
fn general_topic_destination_normalizes_to_chat_scope() {
    let destination = ShareSheetDestination::new("chat-42", Some("  "));

    assert_eq!(destination.chat_id, "chat-42");
    assert_eq!(destination.topic_id, None);
    assert_eq!(destination.id(), "chat-42:");
}

#[test]
fn in_app_text_share_respects_backend_fanout_limit() {
    let destinations = destinations(45);
    let chunks = ShareSheetTextBatchPlanner::chunks(&destinations);

    assert_eq!(chunks.iter().map(Vec::len).collect::<Vec<_>>(), vec![20, 20, 5]);
    assert_eq!(chunks.concat(), destinations);
}

#[test]
fn message_forward_batches_respect_server_copy_limit() {
    let destinations = destinations(45);
    let small_forward = ShareSheetForwardBatchPlanner::chunks(&destinations, 10);
    let bulk_forward = ShareSheetForwardBatchPlanner::chunks(&destinations, 50);

    assert_eq!(small_forward.iter().map(Vec::len).collect::<Vec<_>>(), vec![20, 20, 5]);
    assert_eq!(
        bulk_forward.iter().map(Vec::len).collect::<Vec<_>>(),
        vec![4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 1]
    );
    assert_eq!(bulk_forward.concat(), destinations);
}

#[test]
fn message_forward_response_decodes_destination_results() {
    let response: ForwardMessagesGatewayDTO = decode_fixture(
        r#"{
          "success": true,
          "idempotency_key": "forward-batch-1",
          "all_succeeded": false,
          "results": [{
            "source_message_id": "message-1",
            "destination": {
              "chat_id": "chat-42",
              "topic_id": "topic-7"
            },
            "success": false,
            "error": "forbidden",
            "retryable": false
          }]
        }"#,
    );

    assert_eq!(response.idempotency_key.as_deref(), Some("forward-batch-1"));
    assert_eq!(response.all_succeeded, Some(false));
    let first = response.results.as_ref().and_then(|results| results.first());
    assert_eq!(first.map(|result| result.source_message_id.as_str()), Some("message-1"));
    let destination = first.and_then(|result| result.destination.as_ref());
    assert_eq!(destination.map(|destination| destination.chat_id.as_str()), Some("chat-42"));
    assert_eq!(destination.and_then(|destination| destination.topic_id.as_deref()), Some("topic-7"));
}

#[test]
fn message_share_payload_propagates_target_capabilities() {
    let mut message = GetMessageMessage::new("message-1".to_owned());
    message.server_id = Some("message-1".to_owned());
    message.sender_name = "Arthur".to_owned();
    message.content_type = MessageContentType::Document;
    message.text = "Spec https://riseonly.net/docs/1".to_owned();
    message.media_items = vec![MessageMediaItem::new(
        "document-1".to_owned(),
        MessageMediaKind::Document,
        "https://riseonly.net/files/spec.pdf".to_owned(),
    )];
    let payload = SharePayload::Messages(vec![message]);

    assert!(payload.share_url().is_none());
    assert!(payload.requires_link_permission());
    assert!(!payload.requires_media_permission());
    assert!(payload.requires_documents_permission());
    let forwarded_ids = payload
        .forwarded_messages()
        .map(|messages| messages.iter().map(GetMessageMessage::sai_server_id).collect::<Vec<_>>());
    assert_eq!(forwarded_ids, Some(vec!["message-1".to_owned()]));
    let share_items = payload.share_items();
    assert_eq!(share_items.len(), 1);
    assert!(!share_items.iter().any(|item| matches!(item, ShareItem::Link(_))));
    assert!(
        share_items
            .iter()
            .all(|item| !format!("{item:?}").contains("https://riseonly.net/files/spec.pdf"))
    );
}

#[test]
fn external_message_share_sanitizes_materialized_file_names() {
    let media = MessageMediaItem::new(
        "document-1".to_owned(),
        MessageMediaKind::Document,
        "https://riseonly.net/files/private".to_owned(),
    );
    let source_url = url("https://riseonly.net/api/file/secure/document/token");
    let file_name = MessageExternalShareMaterializer::safe_file_name(
        Some("../../private:report.pdf"),
        &source_url,
        None,
        &media,
        0,
    );

    assert_eq!(file_name, "001-private_report.pdf");
    assert!(!file_name.contains('/'));
    assert!(!file_name.contains(".."));
}

#[test]
fn share_link_permission_uses_the_common_rendered_text_detector() {
    assert!(FormattedText::contains_link("Release notes: https://riseonly.net/p/42"));
    assert!(!FormattedText::contains_link("Release notes without a link"));
}

#[test]
fn share_payload_acknowledgement_decodes_through_sai_snake_case_strategy() {
    let shared: view::Shared = decode_fixture(
        r#"{
          "idempotency_key": "batch-1",
          "all_succeeded": true,
          "results": [{
            "chat_id": "chat-42",
            "topic_id": "topic-7",
            "success": true,
            "error": null,
            "retryable": false,
            "message": {
              "id": "message-7",
              "chat_id": "chat-42",
              "content": "Shared",
              "content_type": "text",
              "created_at": 1783992000000,
              "topic_id": "topic-7"
            }
          }]
        }"#,
    );
    let response = RiseStoredSharePayloadReply::from_canon(&shared);

    assert_eq!(response.idempotency_key, "batch-1");
    assert!(response.all_succeeded);
    let first = response.results.first();
    assert_eq!(first.map(|result| result.chat_id.as_str()), Some("chat-42"));
    assert_eq!(first.and_then(|result| result.topic_id.as_deref()), Some("topic-7"));
    assert_eq!(first.map(|result| result.retryable), Some(false));
    let message = first.and_then(|result| result.message.as_ref());
    assert_eq!(message.and_then(|message| message.id.as_deref()), Some("message-7"));
    assert_eq!(message.and_then(|message| message.topic_id.as_deref()), Some("topic-7"));
}

#[test]
fn second_timestamps_are_normalized_to_milliseconds() {
    let response: GetShareTargetsGatewayDTO = decode_fixture(
        r#"{
          "targets": [{
            "chat_id": "chat-1",
            "title": "Recent",
            "type": "private",
            "can_send": true,
            "last_activity": 1783992000
          }]
        }"#,
    );

    assert_eq!(
        response
            .targets
            .first()
            .and_then(ShareTargetsServicesStore::map_target)
            .map(|target| target.last_activity_ms),
        Some(1_783_992_000_000)
    );
}

#[test]
fn shared_snapshot_round_trips_without_credentials() {
    let snapshot = RiseShareSnapshot::new(
        1_783_992_000_123,
        url("https://riseonly.net/api"),
        url("wss://riseonly.net/"),
        RiseShareAccount {
            id: "user-1".to_owned(),
            name: "Nics".to_owned(),
            tag: "nics51".to_owned(),
            avatar_url: None,
        },
        vec![RiseShareTarget::new(
            "chat-1".to_owned(),
            "Saved".to_owned(),
            None,
            Some("peer-1".to_owned()),
            RiseShareTargetKind::Favourites,
            false,
            true,
            1_783_992_000_000,
        )],
    );

    let data = match serde_json::to_vec(&snapshot) {
        Ok(data) => data,
        Err(error) => panic!("the snapshot does not encode: {error}"),
    };
    let decoded = match serde_json::from_slice::<RiseShareSnapshot>(&data) {
        Ok(decoded) => decoded,
        Err(error) => panic!("the snapshot does not decode: {error}"),
    };

    assert_eq!(decoded, snapshot);
    let text = String::from_utf8_lossy(&data);
    assert!(!text.contains("accessToken"));
    assert!(!text.contains("refreshToken"));
}

#[test]
fn legacy_snapshot_without_avatar_color_key_still_decodes() {
    let decoded: RiseShareSnapshot = decode_fixture(
        r#"{
          "schemaVersion": 1,
          "generatedAtMs": 1783992000123,
          "apiBaseURL": "https://riseonly.net/api",
          "webSocketURL": "wss://riseonly.net/",
          "account": { "id": "account-1", "name": "Nics", "tag": "nics51" },
          "targets": [{
            "id": "chat-1",
            "title": "Legacy",
            "kind": "group",
            "topicsEnabled": false,
            "canSend": true,
            "lastActivityMs": 1783992000000
          }]
        }"#,
    );

    let first = decoded.targets.first();
    assert_eq!(first.map(|target| target.avatar_color_key.is_none()), Some(true));
    assert_eq!(first.map(|target| target.title.as_str()), Some("Legacy"));
}

#[test]
fn story_item_resolution_rejects_traversal() {
    let file_manager = FileManager;
    let directory = temporary_directory().join(format!("rise-share-test-{}", Uuid::new_v4().uuid_string()));
    if let Err(error) = file_manager.create_directory(&directory, false) {
        panic!("the test directory was not created: {error:?}");
    }

    let unsafe_item = story_item("item", RiseShareStoryMediaKind::Image, "../outside.jpg", "image/jpeg");
    let resolved: Result<_, RiseShareContainerError> =
        RiseShareContainer::resolved_story_item_url(&unsafe_item, &directory);
    let _ = file_manager.remove_item(&directory);

    assert!(resolved.is_err());
}

#[test]
fn story_create_encodes_stable_idempotency_contract() {
    let body = rise_canon::rise_request::story::CreateStory {
        allow_screenshots: Some(true),
        audience: Some("everyone".to_owned()),
        caption: Some("hello".to_owned()),
        duration_days: Some(1),
        excluded_user_ids: Some(Vec::new()),
        file_id: Some("file-1".to_owned()),
        idempotency_key: Some("89a20888-ff0e-4308-a568-36e96b4fdf55".to_owned()),
        media_type: Some("image".to_owned()),
        post_to_profile: Some(true),
        selected_user_ids: Some(Vec::new()),
        ..rise_canon::rise_request::story::CreateStory::default()
    };

    let object = match serde_json::to_value(&body) {
        Ok(Value::Object(object)) => object,
        Ok(other) => panic!("the body is not an object: {other}"),
        Err(error) => panic!("the body does not encode: {error}"),
    };
    assert_eq!(
        object.get("idempotency_key").and_then(Value::as_str),
        body.idempotency_key.as_deref()
    );
    assert!(object.get("user_id").is_none());
    assert_eq!(object.get("file_id").and_then(Value::as_str), Some("file-1"));
}

#[test]
fn legacy_story_manifest_defaults_to_first_item_without_checkpoint() {
    let session: RiseShareStorySession = decode_fixture(
        r#"{
          "schemaVersion": 2,
          "id": "session-12345678",
          "accountId": "account-1",
          "createdAtMs": 1783992000123,
          "items": [{
            "id": "item-12345678",
            "kind": "image",
            "relativePath": "item.jpg",
            "suggestedFilename": "item.jpg",
            "contentType": "image/jpeg"
          }]
        }"#,
    );

    assert_eq!(session.next_item_index, 0);
    assert!(session.publish_checkpoint.is_none());
}

#[test]
fn shared_story_checkpoint_round_trips_exact_create_payload() {
    let items = vec![
        story_item("item-12345678", RiseShareStoryMediaKind::Image, "item.jpg", "image/jpeg"),
        story_item("item-87654321", RiseShareStoryMediaKind::Video, "item.mp4", "video/mp4"),
    ];
    let key =
        RiseShareStoryPublishCheckpoint::stable_idempotency_key("account-1", "session-12345678", &items[1].id);
    let checkpoint = RiseShareStoryPublishCheckpoint {
        item_id: items[1].id.clone(),
        idempotency_key: key.clone(),
        remote_file_url: "https://files.riseonly.net/file-1".to_owned(),
        media_type: "video".to_owned(),
        caption: Some("caption".to_owned()),
        audience: "contacts".to_owned(),
        allow_screenshots: false,
        post_to_profile: true,
        duration_days: 2,
        excluded_user_ids: Vec::new(),
        selected_user_ids: Vec::new(),
    };
    let session = RiseShareStorySession::new(
        "session-12345678".to_owned(),
        "account-1".to_owned(),
        1_783_992_000_123,
        items,
        1,
        Some(checkpoint.clone()),
    );

    let data = match serde_json::to_vec(&session) {
        Ok(data) => data,
        Err(error) => panic!("the session does not encode: {error}"),
    };
    let decoded = match serde_json::from_slice::<RiseShareStorySession>(&data) {
        Ok(decoded) => decoded,
        Err(error) => panic!("the session does not decode: {error}"),
    };

    assert_eq!(decoded, session);
    assert_eq!(
        decoded.publish_checkpoint.as_ref().map(|value| value.remote_file_url.as_str()),
        Some(checkpoint.remote_file_url.as_str())
    );
    assert_eq!(
        decoded.publish_checkpoint.as_ref().map(|value| value.idempotency_key.as_str()),
        Some(key.as_str())
    );
}

#[test]
fn shared_story_idempotency_key_is_stable_bounded_and_item_scoped() {
    let long_account = "\u{430}\u{43a}\u{43a}\u{430}\u{443}\u{43d}\u{442}".repeat(100);
    let first =
        RiseShareStoryPublishCheckpoint::stable_idempotency_key(&long_account, "session-12345678", "item-12345678");
    let repeated =
        RiseShareStoryPublishCheckpoint::stable_idempotency_key(&long_account, "session-12345678", "item-12345678");
    let next_item =
        RiseShareStoryPublishCheckpoint::stable_idempotency_key(&long_account, "session-12345678", "item-87654321");

    assert_eq!(first, repeated);
    assert_ne!(first, next_item);
    assert!(first.starts_with("rise-story-v1-"));
    assert!(first.len() <= 128);
}

#[test]
fn share_suggestion_prefers_conversation_over_recipient_identity() {
    let resolved = RiseShareIntentDestinationID::resolve(Some("chat-42"), Some("member-7"), Some("member-handle"));

    assert_eq!(resolved.as_deref(), Some("chat-42"));
}

#[test]
fn incoming_suggestion_requires_exact_account_and_writable_chat() {
    let snapshot = RiseShareSnapshot::new(
        1_783_992_000_123,
        url("https://riseonly.net/api"),
        url("wss://riseonly.net/"),
        RiseShareAccount {
            id: "account-1".to_owned(),
            name: "Nics".to_owned(),
            tag: "nics51".to_owned(),
            avatar_url: None,
        },
        vec![
            target("writable-chat", "Writable", RiseShareTargetKind::Group, true, 1),
            target("read-only-chat", "Read only", RiseShareTargetKind::Channel, false, 1),
        ],
    );

    assert!(RiseShareSuggestionEligibility::can_donate_incoming_message(
        Some(&snapshot),
        Some("account-1"),
        Some("writable-chat")
    ));
    assert!(!RiseShareSuggestionEligibility::can_donate_incoming_message(
        Some(&snapshot),
        Some("account-1"),
        Some("read-only-chat")
    ));
    assert!(!RiseShareSuggestionEligibility::can_donate_incoming_message(
        Some(&snapshot),
        Some("another-account"),
        Some("writable-chat")
    ));
    assert!(!RiseShareSuggestionEligibility::can_donate_incoming_message(
        Some(&snapshot),
        None,
        Some("writable-chat")
    ));
    assert!(!RiseShareSuggestionEligibility::can_donate_incoming_message(
        None,
        Some("account-1"),
        Some("writable-chat")
    ));
}
