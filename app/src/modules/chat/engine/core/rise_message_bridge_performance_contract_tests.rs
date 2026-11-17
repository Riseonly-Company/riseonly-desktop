use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use futures::executor::block_on;
use gpui::{Entity, TestAppContext, px};
use parking_lot::Mutex;
use rise_canon::rise_wire_methods;
use rise_core::foundation::foundation_calendar::{Calendar, TimeZone};
use rise_core::foundation::foundation_date::Date;
use rise_engine::rise_store::rise_store_entities::RiseEntityViewLoadState;
use rise_i18n::I18n;
use rise_i18n::language_store::Locale;
use serde::de::DeserializeOwned;

use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiWsSliceStatus;
use crate::modules::chat::components::chat::async_render::async_text_bubble_types::AsyncTextBubbleLayoutSpec;
#[cfg(debug_assertions)]
use crate::modules::chat::components::chat::async_render::chat_message_render_snapshot::ChatMessageRenderSnapshotBuilder;
use crate::modules::chat::components::chat::async_render::chat_message_render_snapshot::{
    ChatMessageRenderEntryGroupFlags, ChatMessageRenderEntryKind, ChatMessageRenderPreparationSeed,
    ChatMessageRenderPreparationSeedLocalization, ChatMessageRenderRequestKey, ChatMessageRenderRequestPolicy,
    ChatMessageRenderRequestPolicyAction, ChatMessageRenderSnapshotPreparer, RiseMessageGeometryFingerprintPolicy,
};
use crate::modules::chat::components::chat::async_render::rise_message_list_view::{
    RiseMessageNativeLayoutReusePolicy, RiseMessageSnapshotReusePolicy,
};
use crate::modules::chat::components::chat::message_system_pill::MessageSystemFormatContext;
use crate::modules::chat::components::chat::media_viewer::chat_media_gallery_builder::ChatViewLimitedDestructionStrings;
use crate::modules::chat::components::chat_search::chat_search_overlay_store::ChatSearchMessageKind;
use crate::modules::chat::engine::core::rise_chat_overlay_search_presentation::{
    RiseChatOverlaySearchPresentationPreparer, RiseChatOverlaySearchScope, RiseSearchRequestGenerationPolicy,
};
use crate::modules::chat::engine::core::rise_conversation_auxiliary_repository::{
    RiseConversationAuxiliaryLoadState, RiseConversationAuxiliaryPageSnapshot,
};
use crate::modules::chat::engine::core::rise_conversation_entity_records::{
    RiseConversationEntitySource, RiseMessageEntityRecord, RiseMessageLocalPhase,
};
use crate::modules::chat::engine::core::rise_conversation_presentation::{
    RiseConversationEntityStamp, RiseConversationPresentation, RiseMessagePresentationContentStamp,
    RiseMessagePresentationPreparer, RiseMessagePresentationPublishReceipt, RiseMessageScopePresentationSnapshot,
    RiseMessageTabKind, RiseMessageTabPresentationPreparer, RiseMessageTabScope, RiseMessageWireSnapshot,
    RisePinnedMessagesPresentationPreparer,
};
use crate::modules::chat::engine::core::rise_conversation_rpc::{RiseConversationEndpointKind, RiseConversationEndpoints};
use crate::modules::chat::engine::core::rise_global_message_search_presentation::{
    RiseGlobalMessageSearchPresentationPreparer, RiseGlobalMessageSearchScope,
};
use crate::modules::chat::engine::messages::rise_message_contracts::{RiseMessagePageContract, RiseMessageRepositoryError};
use crate::modules::chat::engine::messages::rise_message_scope::{RiseMessagePageLoadOutcome, RiseMessageScope};
use crate::modules::chat::engine::rise_conversation_domain::{
    RiseMessageObservationRecoveryPolicy, RiseMessagePresentationInputPolicy, RiseMessageSynchronizationEpochPolicy,
    RiseMessageSynchronizationFlightPolicy, RiseMessageTimelineRecoveryPolicy,
};
use crate::modules::chat::stores::chats::chat_actions::chat_wire_types::RiseStoredChat;
use crate::modules::chat::stores::message::message_actions::message_types::{
    GetMessageMessage, MessageMediaKind, MessageNormalizedEntities, MessageRoleMentionEntity,
};
use crate::modules::chat::stores::message::message_actions::message_wire_types::MessageWireDTO;
use crate::modules::chat::stores::message::message_services::message_services::MessageServicesStore;
use crate::modules::post::stores::post_actions::post_wire_types::GetFeedPostItemDTO;
use crate::modules::search::stores::search_actions::search_wire_types::SearchResultItemDTO;

struct RiseMessageObservationFlag {
    value: Mutex<bool>,
}

impl RiseMessageObservationFlag {
    fn new() -> Self {
        Self { value: Mutex::new(false) }
    }

    fn mark(&self) {
        *self.value.lock() = true;
    }

    fn is_marked(&self) -> bool {
        *self.value.lock()
    }
}

fn scope(chat_id: &str) -> RiseMessageScope {
    RiseMessageScope::new(chat_id.to_owned(), None, None)
}

fn decode<T: DeserializeOwned>(text: &str) -> T {
    match serde_json::from_str::<T>(text) {
        Ok(value) => value,
        Err(error) => panic!("test payload decodes: {error}"),
    }
}

fn entity_stamp(version: i64, updated_at_milliseconds: i64) -> RiseConversationEntityStamp {
    RiseConversationEntityStamp { version, updated_at_milliseconds }
}

fn content_stamp(version: i64, updated_at_milliseconds: i64) -> RiseMessagePresentationContentStamp {
    RiseMessagePresentationContentStamp {
        entity: entity_stamp(version, updated_at_milliseconds),
        local_revision: 0,
        local_phase: RiseMessageLocalPhase::Remote,
    }
}

fn request_key(account_epoch: &str, source_version: i64, time_zone_identifier: &str) -> ChatMessageRenderRequestKey {
    ChatMessageRenderRequestKey {
        account_user_id: Some("account".to_owned()),
        account_epoch: Some(account_epoch.to_owned()),
        chat_id: "chat-a".to_owned(),
        source_version,
        group_by_topic: false,
        topic_titles: HashMap::new(),
        locale_identifier: "en_US".to_owned(),
        calendar_identifier: "gregorian".to_owned(),
        time_zone_identifier: time_zone_identifier.to_owned(),
        saved_messages_chat: false,
        view_limited_opened: HashSet::new(),
        view_limited_closed: HashSet::new(),
    }
}

fn text_message(id: &str, text: &str, created_at: f64) -> GetMessageMessage {
    let mut message = GetMessageMessage::new(id.to_owned());
    message.chat_id = "chat-a".to_owned();
    message.sender_id = "sender-a".to_owned();
    message.text = text.to_owned();
    message.created_at = created_at;
    message
}

fn format_context() -> MessageSystemFormatContext {
    MessageSystemFormatContext {
        current_user_ids: HashSet::new(),
        strings: HashMap::new(),
        plural_rules: I18n::shared().plural_rules(),
        gift_months_templates: HashMap::new(),
        channel_title: None,
    }
}

fn localization() -> ChatMessageRenderPreparationSeedLocalization {
    ChatMessageRenderPreparationSeedLocalization {
        today: "Today".to_owned(),
        yesterday: "Yesterday".to_owned(),
        day_month_pattern: "MMMM d".to_owned(),
        day_month_year_pattern: "MMMM d, yyyy".to_owned(),
        general_topic: "General".to_owned(),
        unknown_topic: "Topic".to_owned(),
        unread_messages: "Unread messages".to_owned(),
        view_limited: ChatViewLimitedDestructionStrings {
            photo_timed: "Photo".to_owned(),
            photo_once: "Photo".to_owned(),
            video_once: "Video".to_owned(),
        },
    }
}

fn utc_time_zone() -> TimeZone {
    TimeZone::with_seconds_from_gmt(0).expect("zero offset is a valid time zone")
}

fn preparation_seed(
    key: ChatMessageRenderRequestKey,
    messages: Vec<Arc<GetMessageMessage>>,
    content_stamps_by_local_id: HashMap<String, RiseMessagePresentationContentStamp>,
    now: Date,
) -> ChatMessageRenderPreparationSeed {
    let time_zone = utc_time_zone();
    let mut calendar = Calendar::gregorian();
    calendar.time_zone = time_zone.clone();
    ChatMessageRenderPreparationSeed {
        key,
        messages,
        content_stamps_by_local_id,
        system_format_context: format_context(),
        localization: localization(),
        locale: Locale::new("en_US"),
        calendar,
        time_zone,
        now,
        search_highlight: None,
        unread_separator_message_id: None,
    }
}

#[test]
fn synchronization_runs_once_per_lifecycle_epoch_unless_forced() {
    assert!(RiseMessageSynchronizationEpochPolicy::should_synchronize(false, true, None, 7));
    assert!(!RiseMessageSynchronizationEpochPolicy::should_synchronize(false, true, Some(7), 7));
    assert!(RiseMessageSynchronizationEpochPolicy::should_synchronize(false, true, Some(7), 8));
    assert!(RiseMessageSynchronizationEpochPolicy::should_synchronize(true, true, Some(7), 7));
    assert!(RiseMessageSynchronizationEpochPolicy::should_synchronize(false, false, Some(7), 7));
}

#[test]
fn only_authoritative_seed_outcomes_consume_synchronization_epoch() {
    assert!(!RiseMessageSynchronizationEpochPolicy::consumes_epoch(&RiseMessagePageLoadOutcome::AlreadyLoading));
    assert!(RiseMessageSynchronizationEpochPolicy::consumes_epoch(&RiseMessagePageLoadOutcome::Loaded { count: 0 }));
    assert!(!RiseMessageSynchronizationEpochPolicy::consumes_epoch(&RiseMessagePageLoadOutcome::StaleResponseDropped));
}

#[test]
fn empty_timeline_recovers_from_matching_chat_preview_anchor() {
    assert_eq!(
        RiseMessageTimelineRecoveryPolicy::latest_anchor(0, None, None, Some(" message-latest "), Some("topic-a")),
        Some("message-latest".to_owned())
    );
    assert_eq!(RiseMessageTimelineRecoveryPolicy::latest_anchor(1, None, None, Some("message-latest"), None), None);
    assert_eq!(
        RiseMessageTimelineRecoveryPolicy::latest_anchor(0, Some("thread-a"), None, Some("message-latest"), None),
        None
    );
    assert_eq!(
        RiseMessageTimelineRecoveryPolicy::latest_anchor(
            0,
            None,
            Some("topic-a"),
            Some("message-latest"),
            Some("topic-b")
        ),
        None
    );
}

#[test]
fn missing_message_collection_cannot_materialize_an_empty_timeline() {
    let missing = RiseMessagePageContract::require_messages(None).err();
    assert_eq!(
        missing.as_ref().and_then(|error| error.downcast_ref::<RiseMessageRepositoryError>()),
        Some(&RiseMessageRepositoryError::InvalidWirePayload)
    );
    assert!(matches!(RiseMessagePageContract::require_messages(Some(Vec::new())), Ok(messages) if messages.is_empty()));
}

fn empty_loaded_snapshot(account_id: &str, scope: &RiseMessageScope, revision: i64) -> RiseMessageWireSnapshot {
    RiseMessageWireSnapshot {
        account_id: account_id.to_owned(),
        scope: scope.clone(),
        revision,
        rows: Vec::new(),
        local_ids: Vec::new(),
        entity_stamps: Vec::new(),
        anchor_local_id: None,
        has_cached_before: false,
        has_cached_after: false,
        before_cursor: None,
        after_cursor: None,
        has_more_before: false,
        has_more_after: false,
        load_state: RiseEntityViewLoadState::Loaded,
        error_code: None,
    }
}

#[test]
fn identical_observation_seed_is_prepared_only_once() {
    let scope = scope("chat-a");
    let first = empty_loaded_snapshot("account-a", &scope, 7);
    let changed = empty_loaded_snapshot("account-a", &scope, 8);

    assert!(RiseMessagePresentationInputPolicy::is_duplicate(&first, &first));
    assert!(!RiseMessagePresentationInputPolicy::is_duplicate(&first, &changed));
}

#[test]
fn released_or_superseded_seed_cannot_own_the_reopened_scope() {
    assert!(RiseMessageSynchronizationFlightPolicy::accepts(Some(4), Some(9), 4, 4, 9));
    assert!(!RiseMessageSynchronizationFlightPolicy::accepts(Some(4), None, 4, 4, 9));
    assert!(!RiseMessageSynchronizationFlightPolicy::accepts(Some(4), Some(10), 4, 4, 9));
    assert!(!RiseMessageSynchronizationFlightPolicy::accepts(Some(4), Some(9), 5, 4, 9));
}

#[test]
fn observation_recovery_is_bounded_and_generation_owned() {
    assert_eq!(RiseMessageObservationRecoveryPolicy::delay_milliseconds(0), Some(0));
    assert_eq!(RiseMessageObservationRecoveryPolicy::delay_milliseconds(1), Some(250));
    assert_eq!(RiseMessageObservationRecoveryPolicy::delay_milliseconds(2), Some(1_000));
    assert_eq!(RiseMessageObservationRecoveryPolicy::delay_milliseconds(3), Some(4_000));
    assert_eq!(RiseMessageObservationRecoveryPolicy::delay_milliseconds(4), None);
    assert!(RiseMessageObservationRecoveryPolicy::accepts(true, false, true, true, Some(12), 12, false, true));
    assert!(!RiseMessageObservationRecoveryPolicy::accepts(true, false, true, true, Some(13), 12, false, true));
    assert!(!RiseMessageObservationRecoveryPolicy::accepts(true, false, true, false, Some(12), 12, false, true));
    assert!(!RiseMessageObservationRecoveryPolicy::accepts(true, false, false, true, Some(12), 12, false, true));
}

fn topic_titles(title: &str) -> HashMap<String, String> {
    HashMap::from([("topic-a".to_owned(), title.to_owned())])
}

#[test]
fn snapshot_reuse_ignores_unused_topic_titles() {
    assert!(RiseMessageSnapshotReusePolicy::can_reuse(
        "chat-a",
        7,
        false,
        &HashMap::new(),
        "chat-a",
        7,
        false,
        &topic_titles("Changed"),
    ));
}

#[test]
fn snapshot_reuse_requires_exact_active_render_mode() {
    let base = topic_titles("Alpha");

    assert!(RiseMessageSnapshotReusePolicy::can_reuse("chat-a", 7, true, &base, "chat-a", 7, true, &base));
    assert!(!RiseMessageSnapshotReusePolicy::can_reuse(
        "chat-a",
        7,
        true,
        &base,
        "chat-a",
        7,
        true,
        &topic_titles("Beta"),
    ));
    assert!(!RiseMessageSnapshotReusePolicy::can_reuse("chat-a", 7, true, &base, "chat-a", 8, true, &base));
}

#[test]
fn render_request_policy_coalesces_and_rejects_stale_results() {
    let cached = request_key("epoch-1", 7, "UTC");
    let incoming = request_key("epoch-1", 8, "UTC");

    assert_eq!(
        ChatMessageRenderRequestPolicy::action(Some(&cached), None, false, &cached),
        ChatMessageRenderRequestPolicyAction::UseCached
    );
    assert_eq!(
        ChatMessageRenderRequestPolicy::action(Some(&cached), Some(&incoming), true, &incoming),
        ChatMessageRenderRequestPolicyAction::CoalesceInFlight
    );
    assert_eq!(
        ChatMessageRenderRequestPolicy::action(Some(&cached), Some(&cached), true, &incoming),
        ChatMessageRenderRequestPolicyAction::ReplaceInFlight
    );
    assert!(ChatMessageRenderRequestPolicy::accepts_prepared_result(
        4,
        4,
        &incoming,
        Some(&incoming),
        Some("account"),
        Some("epoch-1"),
    ));
    assert!(!ChatMessageRenderRequestPolicy::accepts_prepared_result(
        4,
        5,
        &incoming,
        Some(&incoming),
        Some("account"),
        Some("epoch-1"),
    ));
    assert!(!ChatMessageRenderRequestPolicy::accepts_prepared_result(
        4,
        4,
        &incoming,
        Some(&incoming),
        Some("account"),
        Some("epoch-2"),
    ));
}

#[test]
fn prepared_geometry_uses_content_stamp_and_legacy_content_off_main() {
    let key = request_key("epoch-1", 7, "UTC");
    let mut message = GetMessageMessage::new("message-a".to_owned());
    message.chat_id = "chat-a".to_owned();
    message.sender_id = "sender-a".to_owned();
    message.text = "large-content-".repeat(200);
    let group = ChatMessageRenderEntryGroupFlags { is_first: true, is_last: true };
    let version = |message: &GetMessageMessage, stamp: Option<RiseMessagePresentationContentStamp>| {
        RiseMessageGeometryFingerprintPolicy::version(
            "msg-message-a",
            ChatMessageRenderEntryKind::Message,
            "",
            None,
            Some(message),
            Some(group),
            stamp,
            &key,
        )
        .expect("a message row has a geometry version")
    };

    let first = version(&message, Some(content_stamp(4, 12)));
    let second = version(&message, Some(content_stamp(5, 13)));
    let legacy_first = version(&message, None);
    message.text.push_str("changed");
    let legacy_second = version(&message, None);

    assert!(first != second);
    assert!(legacy_first != legacy_second);
}

#[test]
fn actor_render_preparer_keeps_ordering_and_stable_content_versions() {
    let now = Date::from_time_interval_since_1970(1_750_000_100.0);
    let message = Arc::new(text_message("message-a", "hello", 1_750_000_000.0));
    let stamps = HashMap::from([(message.id.clone(), content_stamp(1, 100))]);
    let preparer = ChatMessageRenderSnapshotPreparer::new();

    let first_seed = preparation_seed(request_key("epoch", 1, "GMT"), vec![message.clone()], stamps.clone(), now);
    let first =
        preparer.prepare(&first_seed, None, false, &|| false).expect("an uncancelled preparation produces a result");
    let second_seed = preparation_seed(request_key("epoch", 2, "GMT"), vec![message], stamps, now);
    let second = preparer
        .prepare(&second_seed, Some(&first.snapshot), false, &|| false)
        .expect("an uncancelled preparation produces a result");

    assert_eq!(
        first.snapshot.entries.iter().map(|entry| entry.kind).collect::<Vec<_>>(),
        vec![ChatMessageRenderEntryKind::DateSeparator, ChatMessageRenderEntryKind::Message]
    );
    assert_eq!(first.snapshot.entries.first().map(|entry| entry.label.as_str()), Some("Today"));
    assert_eq!(
        first.snapshot.entries.iter().map(|entry| entry.content_version).collect::<Vec<_>>(),
        second.snapshot.entries.iter().map(|entry| entry.content_version).collect::<Vec<_>>()
    );
    assert_eq!(second.snapshot.source_version, 2);
}

#[test]
fn actor_render_preparer_rebuilds_only_changed_row_neighborhood() {
    let now = Date::from_time_interval_since_1970(1_750_000_100.0);
    let messages: Vec<GetMessageMessage> = (0..5)
        .map(|index| {
            text_message(&format!("message-{index}"), &format!("text-{index}"), 1_750_000_000.0 + f64::from(index))
        })
        .collect();
    let first_stamps: HashMap<String, RiseMessagePresentationContentStamp> = messages
        .iter()
        .enumerate()
        .map(|(index, message)| {
            let version = index as i64 + 1;
            (message.id.clone(), content_stamp(version, version))
        })
        .collect();
    let preparer = ChatMessageRenderSnapshotPreparer::new();
    let first_seed = preparation_seed(
        request_key("epoch", 1, "GMT"),
        messages.iter().cloned().map(Arc::new).collect(),
        first_stamps.clone(),
        now,
    );
    let first =
        preparer.prepare(&first_seed, None, true, &|| false).expect("an uncancelled preparation produces a result");

    let mut changed_messages = messages;
    changed_messages[2].text = "changed".to_owned();
    let mut changed_stamps = first_stamps;
    changed_stamps.insert(changed_messages[2].id.clone(), content_stamp(99, 99));
    let second_seed = preparation_seed(
        request_key("epoch", 2, "GMT"),
        changed_messages.into_iter().map(Arc::new).collect(),
        changed_stamps,
        now,
    );
    let second = preparer
        .prepare(&second_seed, Some(&first.snapshot), true, &|| false)
        .expect("an uncancelled preparation produces a result");

    assert_eq!(second.metrics.reused_message_bundles, 2);
    assert_eq!(second.metrics.rebuilt_message_bundles, 3);
    assert_eq!(second.metrics.deep_message_comparisons, 0);
    assert!(second.snapshot.delta.inserted_ids.is_empty());
    assert!(second.snapshot.delta.removed_ids.is_empty());
    assert_eq!(second.snapshot.delta.updated_ids, HashSet::from(["msg-message-2".to_owned()]));
}

#[test]
fn message_entities_normalize_custom_emoji_and_role_mentions_in_one_pass() {
    let entities = r#"
    [
      {
        "type": "role_mention",
        "role_id": "role-a",
        "offset": 0,
        "length": 6
      },
      {
        "type": "custom_emoji",
        "custom_emoji_id": "emoji-a",
        "file_url": "https://cdn.example/emoji.tgs",
        "asset_type": "lottie",
        "offset": 7,
        "length": 2
      }
    ]
    "#;
    let normalized = MessageNormalizedEntities::parse(Some(entities));

    assert_eq!(
        normalized.role_mentions,
        vec![MessageRoleMentionEntity { offset: 0, length: 6, role_id: "role-a".to_owned() }]
    );
    assert_eq!(normalized.custom_emojis.len(), 1);
    assert_eq!(normalized.custom_emojis.first().map(|emoji| emoji.custom_emoji_id.as_str()), Some("emoji-a"));
    assert_eq!(normalized.custom_emojis.first().map(|emoji| emoji.asset_type.as_str()), Some("lottie"));
}

#[test]
fn render_entities_never_decode_raw_json_fallback() {
    let mut message = GetMessageMessage::new("message-a".to_owned());
    message.entities = Some(
        r#"[{
          "type": "custom_emoji",
          "custom_emoji_id": "emoji-a",
          "offset": 0,
          "length": 1
        }]"#
            .to_owned(),
    );

    assert!(message.normalized_entities.is_none());
    assert_eq!(message.resolved_entities(), &MessageNormalizedEntities::EMPTY);
}

#[cfg(debug_assertions)]
#[gpui::test]
fn equivalent_snapshot_content_survives_source_version_changes(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let mut message = text_message("stable", "unchanged", 1_750_000_000.0);
        let first = ChatMessageRenderSnapshotBuilder::make(
            vec![Arc::new(message.clone())],
            1,
            HashMap::new(),
            false,
            HashMap::new(),
            false,
            None,
            cx,
        );
        let unchanged = ChatMessageRenderSnapshotBuilder::make(
            vec![Arc::new(message.clone())],
            2,
            HashMap::new(),
            false,
            HashMap::new(),
            false,
            Some(&first),
            cx,
        );
        message.text = "changed".to_owned();
        let changed = ChatMessageRenderSnapshotBuilder::make(
            vec![Arc::new(message)],
            3,
            HashMap::new(),
            false,
            HashMap::new(),
            false,
            Some(&unchanged),
            cx,
        );

        assert!(RiseMessageSnapshotReusePolicy::has_equivalent_rendered_content(&first, &unchanged));
        assert!(!RiseMessageSnapshotReusePolicy::has_equivalent_rendered_content(&unchanged, &changed));
    });
}

#[cfg(debug_assertions)]
#[gpui::test]
fn entity_stamp_is_the_authoritative_row_content_identity(cx: &mut TestAppContext) {
    use crate::modules::chat::components::chat::async_render::chat_message_render_snapshot::ChatMessageRenderSnapshot;

    fn message_content_version(snapshot: &ChatMessageRenderSnapshot) -> Option<i64> {
        snapshot
            .entries
            .iter()
            .find(|entry| entry.kind == ChatMessageRenderEntryKind::Message)
            .map(|entry| entry.content_version)
    }

    cx.update(|cx| {
        let mut message = text_message("stamped", "before", 1_750_000_000.0);
        let first_stamp = content_stamp(1, 100);
        let stamps = HashMap::from([(message.id.clone(), first_stamp)]);

        let first = ChatMessageRenderSnapshotBuilder::make(
            vec![Arc::new(message.clone())],
            1,
            stamps.clone(),
            false,
            HashMap::new(),
            false,
            None,
            cx,
        );
        message.text = "mutated-copy-with-same-store-version".to_owned();
        let same_stamp = ChatMessageRenderSnapshotBuilder::make(
            vec![Arc::new(message.clone())],
            2,
            stamps,
            false,
            HashMap::new(),
            false,
            Some(&first),
            cx,
        );
        let changed_stamps = HashMap::from([(message.id.clone(), content_stamp(2, 200))]);
        let changed_stamp = ChatMessageRenderSnapshotBuilder::make(
            vec![Arc::new(message)],
            3,
            changed_stamps,
            false,
            HashMap::new(),
            false,
            Some(&same_stamp),
            cx,
        );

        assert_eq!(message_content_version(&first), message_content_version(&same_stamp));
        assert_ne!(message_content_version(&changed_stamp), message_content_version(&same_stamp));
    });
}

#[test]
fn native_layout_reuse_requires_stable_geometry_inputs() {
    let spec = AsyncTextBubbleLayoutSpec { maximum_width: px(280.0), ..Default::default() };
    let narrower = AsyncTextBubbleLayoutSpec { maximum_width: px(260.0), ..Default::default() };
    let almaty = "Asia/Almaty";

    assert!(RiseMessageNativeLayoutReusePolicy::can_reuse(42, Some(42), &spec, &spec, almaty, almaty, Some(true), true));
    assert!(!RiseMessageNativeLayoutReusePolicy::can_reuse(43, Some(42), &spec, &spec, almaty, almaty, Some(true), true));
    assert!(!RiseMessageNativeLayoutReusePolicy::can_reuse(
        42,
        Some(42),
        &spec,
        &narrower,
        almaty,
        almaty,
        Some(true),
        true,
    ));
    assert!(!RiseMessageNativeLayoutReusePolicy::can_reuse(42, Some(42), &spec, &spec, almaty, "UTC", Some(true), true));
    assert!(!RiseMessageNativeLayoutReusePolicy::can_reuse(
        42,
        Some(42),
        &spec,
        &spec,
        almaty,
        almaty,
        Some(true),
        false,
    ));
}

fn activated_presentation(account_id: &str, cx: &mut TestAppContext) -> Entity<RiseConversationPresentation> {
    cx.update(|cx| {
        let presentation = RiseConversationPresentation::shared(cx);
        presentation.update(cx, |presentation, cx| presentation.activate(account_id, cx));
        presentation
    })
}

fn deactivate(presentation: &Entity<RiseConversationPresentation>, cx: &mut TestAppContext) {
    cx.update(|cx| presentation.update(cx, |presentation, cx| presentation.deactivate(cx)));
}

fn publish_scope(
    presentation: &Entity<RiseConversationPresentation>,
    snapshot: &Arc<RiseMessageScopePresentationSnapshot>,
    cx: &mut TestAppContext,
) -> Option<RiseMessagePresentationPublishReceipt> {
    let snapshot = snapshot.clone();
    cx.update(|cx| presentation.update(cx, |presentation, cx| presentation.publish_message_scope(snapshot, cx)))
}

fn message_ids(messages: &[GetMessageMessage]) -> Vec<String> {
    messages.iter().map(|message| message.id.clone()).collect()
}

fn scope_message_ids(
    presentation: &Entity<RiseConversationPresentation>,
    scope: &RiseMessageScope,
    cx: &mut TestAppContext,
) -> Vec<String> {
    cx.update(|cx| message_ids(presentation.read(cx).messages(scope)))
}

fn scope_revision(
    presentation: &Entity<RiseConversationPresentation>,
    scope: &RiseMessageScope,
    cx: &mut TestAppContext,
) -> u64 {
    cx.update(|cx| presentation.read(cx).message_revision_for(scope))
}

fn make_snapshot(account_id: &str, scope: &RiseMessageScope, local_id: &str, revision: i64) -> RiseMessageWireSnapshot {
    let wire: MessageWireDTO = decode(&format!(
        r#"{{
          "id": "{local_id}",
          "chatId": "{chat_id}",
          "senderId": "sender",
          "content": "hello",
          "contentType": "text",
          "createdAt": 1750000000
        }}"#,
        chat_id = scope.chat_id,
    ));
    RiseMessageWireSnapshot {
        account_id: account_id.to_owned(),
        scope: scope.clone(),
        revision,
        rows: vec![RiseMessageEntityRecord::new(wire, RiseConversationEntitySource::ServerPage)],
        local_ids: vec![local_id.to_owned()],
        entity_stamps: vec![entity_stamp(revision, revision)],
        anchor_local_id: Some(local_id.to_owned()),
        has_cached_before: false,
        has_cached_after: false,
        before_cursor: None,
        after_cursor: None,
        has_more_before: false,
        has_more_after: false,
        load_state: RiseEntityViewLoadState::Loaded,
        error_code: None,
    }
}

fn prepared_snapshot(
    preparer: &RiseMessagePresentationPreparer,
    snapshot: RiseMessageWireSnapshot,
) -> Arc<RiseMessageScopePresentationSnapshot> {
    block_on(preparer.prepare(snapshot, true)).expect("a newer source prepares a snapshot")
}

fn message_wire(id: &str, chat_id: &str, is_pinned: bool, force_reply: bool) -> MessageWireDTO {
    decode(&format!(
        r#"{{
          "id": "{id}",
          "chatId": "{chat_id}",
          "senderId": "sender",
          "content": "hello",
          "contentType": "text",
          "createdAt": 1750000000,
          "isPinned": {is_pinned},
          "replyMarkup": {{
            "forceReply": {force_reply}
          }}
        }}"#
    ))
}

fn media_wire(id: &str, chat_id: &str) -> MessageWireDTO {
    decode(&format!(
        r#"{{
          "id": "{id}",
          "chatId": "{chat_id}",
          "senderId": "sender",
          "contentType": "image",
          "createdAt": 1750000000,
          "mediaItems": [
            {{
              "mediaType": "image",
              "fileId": "file-a",
              "fileUrl": "https://cdn.example/a.jpg",
              "localSlotIndex": 0
            }},
            {{
              "mediaType": "image",
              "fileId": "file-b",
              "fileUrl": "https://cdn.example/b.jpg",
              "localSlotIndex": 1
            }}
          ]
        }}"#
    ))
}

fn entity_record(
    wire: MessageWireDTO,
    source: RiseConversationEntitySource,
    local_phase: RiseMessageLocalPhase,
) -> RiseMessageEntityRecord {
    let mut record = RiseMessageEntityRecord::new(wire, source);
    record.local_phase = local_phase;
    record
}

fn media_snapshot(
    account_id: &str,
    scope: &RiseMessageScope,
    wire: &MessageWireDTO,
    phase: RiseMessageLocalPhase,
    revision: i64,
) -> RiseMessageWireSnapshot {
    let source = if phase == RiseMessageLocalPhase::Remote {
        RiseConversationEntitySource::ServerPage
    } else {
        RiseConversationEntitySource::Optimistic
    };
    RiseMessageWireSnapshot {
        account_id: account_id.to_owned(),
        scope: scope.clone(),
        revision,
        rows: vec![entity_record(wire.clone(), source, phase)],
        local_ids: vec!["message-media".to_owned()],
        entity_stamps: vec![entity_stamp(revision, revision)],
        anchor_local_id: Some("message-media".to_owned()),
        has_cached_before: false,
        has_cached_after: false,
        before_cursor: None,
        after_cursor: None,
        has_more_before: false,
        has_more_after: false,
        load_state: RiseEntityViewLoadState::Loaded,
        error_code: None,
    }
}

#[gpui::test]
async fn first_scope_publication_invalidates_an_initially_empty_reader(cx: &mut TestAppContext) {
    let account_id = "presentation-first-publication";
    let scope = scope("chat-first-publication");
    let preparer = RiseMessagePresentationPreparer::new();
    let invalidation = Arc::new(RiseMessageObservationFlag::new());
    let presentation = activated_presentation(account_id, cx);

    cx.update(|cx| {
        assert!(presentation.read(cx).message_scope_snapshot(&scope).is_none());
        assert!(presentation.read(cx).messages(&scope).is_empty());
        let invalidation = invalidation.clone();
        cx.observe(&presentation, move |_, _| invalidation.mark()).detach();
    });

    let prepared = preparer
        .prepare(make_snapshot(account_id, &scope, "first-message", 1), true)
        .await
        .expect("a newer source prepares a snapshot");
    assert!(publish_scope(&presentation, &prepared, cx).is_some());
    assert!(invalidation.is_marked());
    assert_eq!(scope_message_ids(&presentation, &scope, cx), vec!["first-message".to_owned()]);
    deactivate(&presentation, cx);
}

#[gpui::test]
async fn message_publish_receipt_identifies_only_the_snapshot_actually_accepted(cx: &mut TestAppContext) {
    let account_id = "presentation-publish-receipt";
    let scope = scope("chat-receipt");
    let preparer = RiseMessagePresentationPreparer::new();
    let presentation = activated_presentation(account_id, cx);

    let older = prepared_snapshot(&preparer, make_snapshot(account_id, &scope, "message-older", 41));
    let newer = prepared_snapshot(&preparer, make_snapshot(account_id, &scope, "message-newer", 42));

    let accepted = publish_scope(&presentation, &newer, cx).expect("the newest snapshot is accepted");
    assert!(
        accepted
            == RiseMessagePresentationPublishReceipt {
                account_id: account_id.to_owned(),
                scope: scope.clone(),
                source_revision: 42,
                presentation_revision: newer.revision,
            }
    );
    assert!(publish_scope(&presentation, &newer, cx).is_none());
    assert!(publish_scope(&presentation, &older, cx).is_none());
    let state_revision = cx.update(|cx| presentation.read(cx).message_state(&scope).map(|state| state.revision));
    assert_eq!(state_revision, Some(42));
    assert_eq!(scope_message_ids(&presentation, &scope, cx), vec!["message-newer".to_owned()]);
    deactivate(&presentation, cx);
}

#[gpui::test]
async fn message_publish_rejects_another_account_without_returning_foreign_receipt(cx: &mut TestAppContext) {
    let scope = scope("chat-account-guard");
    let preparer = RiseMessagePresentationPreparer::new();
    let presentation = activated_presentation("presentation-active-account", cx);

    let foreign =
        prepared_snapshot(&preparer, make_snapshot("presentation-foreign-account", &scope, "foreign-message", 99));

    assert!(publish_scope(&presentation, &foreign, cx).is_none());
    assert!(cx.update(|cx| presentation.read(cx).message_state(&scope).is_none()));
    assert_eq!(scope_revision(&presentation, &scope, cx), 0);
    deactivate(&presentation, cx);
}

#[gpui::test]
async fn conditional_scope_removal_cannot_delete_a_reopened_presentation(cx: &mut TestAppContext) {
    let account_id = "presentation-conditional-remove";
    let scope = scope("chat-conditional-remove");
    let preparer = RiseMessagePresentationPreparer::new();
    let presentation = activated_presentation(account_id, cx);

    let original = prepared_snapshot(&preparer, make_snapshot(account_id, &scope, "message-original", 1));
    let reopened = prepared_snapshot(&preparer, make_snapshot(account_id, &scope, "message-reopened", 2));
    assert!(publish_scope(&presentation, &original, cx).is_some());
    assert!(publish_scope(&presentation, &reopened, cx).is_some());

    let remove = |revision: u64, cx: &mut TestAppContext| {
        cx.update(|cx| {
            presentation
                .update(cx, |presentation, cx| presentation.remove_message_scope_if_revision(&scope, Some(revision), cx))
        })
    };
    assert!(!remove(original.revision, cx));
    assert_eq!(scope_revision(&presentation, &scope, cx), reopened.revision);
    assert_eq!(scope_message_ids(&presentation, &scope, cx), vec!["message-reopened".to_owned()]);

    assert!(remove(reopened.revision, cx));
    assert!(cx.update(|cx| presentation.read(cx).message_scope_snapshot(&scope).is_none()));
    assert_eq!(scope_revision(&presentation, &scope, cx), 0);
    deactivate(&presentation, cx);
}

#[test]
fn conditional_preparer_release_cannot_delete_reopened_state() {
    let account_id = "preparer-conditional-release";
    let scope = scope("chat-conditional-release");
    let preparer = RiseMessagePresentationPreparer::new();

    let original = prepared_snapshot(&preparer, make_snapshot(account_id, &scope, "message-original", 1));
    let reopened = prepared_snapshot(&preparer, make_snapshot(account_id, &scope, "message-reopened", 2));

    assert!(!block_on(preparer.release_if_revision(&scope, Some(original.revision))));
    let retained = prepared_snapshot(&preparer, make_snapshot(account_id, &scope, "message-retained", 3));
    assert_eq!(retained.revision, reopened.revision + 1);

    assert!(block_on(preparer.release_if_revision(&scope, Some(retained.revision))));
    assert!(block_on(preparer.hide_outgoing_message("message-retained")).is_empty());
    let fresh = prepared_snapshot(&preparer, make_snapshot(account_id, &scope, "message-fresh", 4));
    assert_eq!(fresh.revision, 1);
}

#[gpui::test]
async fn publishing_one_scope_does_not_advance_another_scope_revision(cx: &mut TestAppContext) {
    let account_id = "presentation-scope-isolation";
    let first_scope = scope("chat-a");
    let second_scope = scope("chat-b");
    let preparer = RiseMessagePresentationPreparer::new();
    let presentation = activated_presentation(account_id, cx);

    let first_prepared = prepared_snapshot(&preparer, make_snapshot(account_id, &first_scope, "message-a", 1));
    publish_scope(&presentation, &first_prepared, cx);
    let first_revision = scope_revision(&presentation, &first_scope, cx);
    assert_eq!(first_revision, 1);
    assert_eq!(scope_revision(&presentation, &second_scope, cx), 0);

    let second_prepared = prepared_snapshot(&preparer, make_snapshot(account_id, &second_scope, "message-b", 1));
    publish_scope(&presentation, &second_prepared, cx);

    assert_eq!(scope_revision(&presentation, &first_scope, cx), first_revision);
    assert_eq!(scope_revision(&presentation, &second_scope, cx), 1);
    assert_eq!(scope_message_ids(&presentation, &first_scope, cx), vec!["message-a".to_owned()]);
    assert_eq!(scope_message_ids(&presentation, &second_scope, cx), vec!["message-b".to_owned()]);
    deactivate(&presentation, cx);
}

#[gpui::test]
async fn scope_snapshot_publishes_derived_chat_values_atomically(cx: &mut TestAppContext) {
    let account_id = "presentation-derived-scope";
    let scope = scope("chat-derived");
    let pinned_wire = message_wire("message-a", &scope.chat_id, true, false);
    let reply_wire = message_wire("message-b", &scope.chat_id, false, true);
    let source = RiseMessageWireSnapshot {
        account_id: account_id.to_owned(),
        scope: scope.clone(),
        revision: 9,
        rows: vec![
            RiseMessageEntityRecord::new(pinned_wire, RiseConversationEntitySource::ServerPage),
            RiseMessageEntityRecord::new(reply_wire, RiseConversationEntitySource::ServerPage),
        ],
        local_ids: vec!["message-a".to_owned(), "message-b".to_owned()],
        entity_stamps: vec![entity_stamp(1, 1), entity_stamp(2, 2)],
        anchor_local_id: Some("message-b".to_owned()),
        has_cached_before: true,
        has_cached_after: false,
        before_cursor: Some("before".to_owned()),
        after_cursor: None,
        has_more_before: true,
        has_more_after: false,
        load_state: RiseEntityViewLoadState::Loaded,
        error_code: None,
    };
    let preparer = RiseMessagePresentationPreparer::new();
    let prepared = prepared_snapshot(&preparer, source);
    let presentation = activated_presentation(account_id, cx);

    publish_scope(&presentation, &prepared, cx);

    let engine_snapshot = cx
        .update(|cx| presentation.read(cx).message_scope_snapshot(&scope))
        .expect("the published scope has a snapshot");
    let ui_snapshot = cx
        .update(|cx| {
            MessageServicesStore::shared(cx)
                .update(cx, |store, cx| store.message_scope_presentation_snapshot(&scope.storage_key(), cx))
        })
        .expect("the store projects the published scope");
    assert_eq!(engine_snapshot.anchor_offset, 1);
    assert_eq!(message_ids(&engine_snapshot.pinned_messages), vec!["message-a".to_owned()]);
    assert_eq!(engine_snapshot.latest_reply_keyboard_message.as_ref().map(|message| message.id.as_str()), Some("message-b"));
    assert_eq!(ui_snapshot.latest_message.as_ref().map(|message| message.id.as_str()), Some("message-b"));
    assert_eq!(ui_snapshot.slice.state.anchor_offset, 1);
    assert_eq!(ui_snapshot.slice.state.generation, engine_snapshot.revision);
    assert!(ui_snapshot.has_more_before);
    assert!(!ui_snapshot.has_more_after);
    deactivate(&presentation, cx);
}

#[gpui::test]
async fn local_hide_is_prepared_off_main_and_rejects_older_commit(cx: &mut TestAppContext) {
    let account_id = "presentation-local-hide";
    let scope = scope("chat-hide");
    let source = RiseMessageWireSnapshot {
        account_id: account_id.to_owned(),
        scope: scope.clone(),
        revision: 3,
        rows: vec![
            entity_record(
                message_wire("message-a", &scope.chat_id, true, false),
                RiseConversationEntitySource::ServerPage,
                RiseMessageLocalPhase::Remote,
            ),
            entity_record(
                message_wire("message-b", &scope.chat_id, false, true),
                RiseConversationEntitySource::Optimistic,
                RiseMessageLocalPhase::Sending,
            ),
        ],
        local_ids: vec!["message-a".to_owned(), "message-b".to_owned()],
        entity_stamps: vec![entity_stamp(1, 1), entity_stamp(2, 2)],
        anchor_local_id: Some("message-b".to_owned()),
        has_cached_before: false,
        has_cached_after: false,
        before_cursor: None,
        after_cursor: None,
        has_more_before: false,
        has_more_after: false,
        load_state: RiseEntityViewLoadState::Loaded,
        error_code: None,
    };
    let preparer = RiseMessagePresentationPreparer::new();
    let initial = prepared_snapshot(&preparer, source);
    let hidden = preparer
        .hide_outgoing_message("message-b")
        .await
        .into_iter()
        .next()
        .expect("hiding a visible row republishes its scope");

    assert_eq!(initial.revision, 1);
    assert_eq!(hidden.revision, 2);
    assert_eq!(message_ids(&hidden.messages), vec!["message-a".to_owned()]);
    assert_eq!(hidden.visible_local_ids, vec!["message-a".to_owned()]);
    assert_eq!(hidden.content_stamps_by_local_id.len(), 1);
    assert_eq!(message_ids(&hidden.pinned_messages), vec!["message-a".to_owned()]);
    assert!(hidden.latest_reply_keyboard_message.is_none());
    assert_eq!(hidden.anchor_offset, 0);

    let presentation = activated_presentation(account_id, cx);
    publish_scope(&presentation, &hidden, cx);
    publish_scope(&presentation, &initial, cx);

    assert_eq!(scope_message_ids(&presentation, &scope, cx), vec!["message-a".to_owned()]);
    assert_eq!(scope_revision(&presentation, &scope, cx), hidden.revision);
    deactivate(&presentation, cx);
}

fn first_message_cancelled_flags(snapshot: &RiseMessageScopePresentationSnapshot) -> Option<Vec<bool>> {
    snapshot.messages.first().map(|message| message.media_items.iter().map(|item| item.is_cancelled).collect())
}

fn media_local_revision(snapshot: &RiseMessageScopePresentationSnapshot) -> Option<u64> {
    snapshot.content_stamps_by_local_id.get("message-media").map(|stamp| stamp.local_revision)
}

#[test]
fn local_media_cancellation_reconciles_with_authoritative_remote() {
    let account_id = "presentation-media-cancel";
    let scope = scope("chat-media");
    let wire = media_wire("message-media", &scope.chat_id);
    let preparer = RiseMessagePresentationPreparer::new();
    let uploading =
        prepared_snapshot(&preparer, media_snapshot(account_id, &scope, &wire, RiseMessageLocalPhase::Uploading, 1));
    let cancelled = block_on(preparer.hide_outgoing_media_slot("message-media", 1))
        .into_iter()
        .next()
        .expect("cancelling a visible slot republishes its scope");
    let authoritative =
        prepared_snapshot(&preparer, media_snapshot(account_id, &scope, &wire, RiseMessageLocalPhase::Remote, 2));

    assert_eq!(uploading.messages.first().map(|message| message.media_items.len()), Some(2));
    assert_eq!(first_message_cancelled_flags(&cancelled), Some(vec![false, true]));
    assert_eq!(media_local_revision(&cancelled), Some(1));
    assert_eq!(first_message_cancelled_flags(&authoritative), Some(vec![false, false]));
    assert_eq!(media_local_revision(&authoritative), Some(0));
    assert!(authoritative.revision > cancelled.revision);
}

#[test]
fn warm_history_deletion_respects_store_revision() {
    for deletion_revision in [1_i64, 3_i64] {
        let account_id = "warm-history-deletion";
        let scope = scope("warm-history-chat");
        let preparer = RiseMessagePresentationPreparer::new();
        block_on(preparer.prepare_warm_snapshot(make_snapshot(account_id, &scope, "message-current", 2)));
        assert!(block_on(preparer.has_warm_snapshot(&scope)));

        block_on(preparer.prepare_warm_snapshot(empty_loaded_snapshot(account_id, &scope, deletion_revision)));

        assert_eq!(block_on(preparer.has_warm_snapshot(&scope)), deletion_revision < 2);
    }
}

fn auxiliary_page<Item>(
    items: Vec<Item>,
    cursor: Option<&str>,
    has_more: bool,
    revision: i64,
    load_state: RiseConversationAuxiliaryLoadState,
    is_persisted: bool,
) -> RiseConversationAuxiliaryPageSnapshot<Item> {
    RiseConversationAuxiliaryPageSnapshot {
        items,
        cursor: cursor.map(str::to_owned),
        has_more,
        revision,
        load_state,
        error: None,
        is_persisted,
    }
}

fn pinned_message_wire(id: &str, created_at: f64, entities: Option<&str>) -> MessageWireDTO {
    let entities = match entities {
        Some(entities) => {
            let encoded = serde_json::to_string(entities).expect("a string always encodes");
            format!(r#","entities":{encoded}"#)
        }
        None => String::new(),
    };
    decode(&format!(
        r#"{{"chatId":"chat-a","content":"hello","contentType":"text","createdAt":{created_at:?},"id":"{id}","senderId":"sender"{entities}}}"#
    ))
}

#[gpui::test]
async fn pinned_page_is_prepared_before_one_scoped_presentation_commit(cx: &mut TestAppContext) {
    let first = pinned_message_wire("message-a", 1_750_000_100.0, None);
    let second = pinned_message_wire(
        "message-b",
        1_750_000_000.0,
        Some(r#"[{"type":"role_mention","role_id":"role-a","offset":0,"length":4}]"#),
    );
    let preparer = RisePinnedMessagesPresentationPreparer::new();
    let prepared = preparer
        .prepare(
            "pinned-presentation-account",
            "chat-a",
            &auxiliary_page(
                vec![first.clone(), second],
                Some("message-a"),
                true,
                7,
                RiseConversationAuxiliaryLoadState::Loaded,
                true,
            ),
        )
        .await;
    let presentation = activated_presentation(&prepared.account_id, cx);
    cx.update(|cx| presentation.update(cx, |presentation, cx| presentation.publish_pinned_messages(prepared, cx)));

    cx.update(|cx| {
        let presentation = presentation.read(cx);
        assert_eq!(message_ids(presentation.pinned_messages("chat-a")), vec!["message-b".to_owned(), "message-a".to_owned()]);
        let role_id = presentation
            .pinned_messages("chat-a")
            .first()
            .and_then(|message| message.normalized_entities.as_ref())
            .and_then(|entities| entities.role_mentions.first())
            .map(|mention| mention.role_id.as_str());
        assert_eq!(role_id, Some("role-a"));
        assert_eq!(presentation.pinned_messages_cursor("chat-a"), Some("message-a"));
        assert!(presentation.pinned_messages_has_more("chat-a"));
        assert!(presentation.pinned_messages_has_resolved_initial("chat-a"));
    });

    let revision = cx.update(|cx| presentation.read(cx).pinned_messages_revision("chat-a"));
    let updated = preparer
        .prepare(
            "pinned-presentation-account",
            "chat-a",
            &auxiliary_page(vec![first], None, false, 8, RiseConversationAuxiliaryLoadState::Loaded, true),
        )
        .await;
    cx.update(|cx| presentation.update(cx, |presentation, cx| presentation.publish_pinned_messages(updated, cx)));
    cx.update(|cx| {
        let presentation = presentation.read(cx);
        assert_eq!(message_ids(presentation.pinned_messages("chat-a")), vec!["message-a".to_owned()]);
        assert_eq!(presentation.pinned_messages_revision("chat-a"), revision + 1);
    });
    deactivate(&presentation, cx);
}

fn tab_message_wire(id: &str, content_type: &str, media_type: &str, created_at: f64) -> MessageWireDTO {
    decode(&format!(
        r#"{{
          "id": "{id}",
          "chat_id": "chat-a",
          "sender_id": "sender-a",
          "content_type": "{content_type}",
          "created_at": {created_at:?},
          "media_items": [{{
            "file_id": "{id}-media",
            "file_url": "https://cdn.example/{id}",
            "media_type": "{media_type}"
          }}]
        }}"#
    ))
}

#[test]
fn legacy_tab_identifiers_map_to_typed_kinds() {
    assert_eq!(RiseMessageTabKind::from_cache_id("getMediaMessages"), Some(RiseMessageTabKind::Media));
    assert_eq!(RiseMessageTabKind::from_cache_id("get_file_messages"), Some(RiseMessageTabKind::Files));
    assert_eq!(RiseMessageTabKind::from_cache_id("get_music_messages"), Some(RiseMessageTabKind::Music));
    assert_eq!(RiseMessageTabKind::from_cache_id("get_audio_messages"), Some(RiseMessageTabKind::Voice));
    assert_eq!(RiseMessageTabKind::from_cache_id("get_gif_messages"), Some(RiseMessageTabKind::Gif));
    assert_eq!(RiseMessageTabKind::from_cache_id("get_fav_messages"), Some(RiseMessageTabKind::Favorites));
    assert_eq!(RiseMessageTabKind::from_cache_id("getLinkMessages"), Some(RiseMessageTabKind::Links));
    assert_eq!(RiseMessageTabKind::from_cache_id("unknown"), None);
}

#[test]
fn storage_scope_preserves_topic_and_rejects_thread_roots() {
    let topic_scope = RiseMessageScope::new("chat-a".to_owned(), Some("topic-a".to_owned()), None);
    let tab_scope = RiseMessageTabScope::from_storage_scope(RiseMessageTabKind::Files, &topic_scope.storage_key());
    assert_eq!(tab_scope.as_ref().map(|scope| scope.chat_id.as_str()), Some("chat-a"));
    assert_eq!(tab_scope.as_ref().and_then(|scope| scope.topic_id.as_deref()), Some("topic-a"));

    let thread_scope = RiseMessageScope::new("chat-a".to_owned(), None, Some("thread-a".to_owned()));
    assert!(RiseMessageTabScope::from_storage_scope(RiseMessageTabKind::Files, &thread_scope.storage_key()).is_none());
}

#[gpui::test]
async fn message_page_is_filtered_and_prepared_before_presentation_commit(cx: &mut TestAppContext) {
    let file = tab_message_wire("file-message", "document", "document", 1_750_000_000.0);
    let voice = tab_message_wire("voice-message", "voice", "voice", 1_750_000_100.0);
    let page = auxiliary_page(
        vec![file, voice],
        Some("next-message"),
        true,
        11,
        RiseConversationAuxiliaryLoadState::Loaded,
        true,
    );
    let account_id = "message-tab-account";
    let preparer = RiseMessageTabPresentationPreparer::new();
    let file_scope = RiseMessageTabScope::new(RiseMessageTabKind::Files, "chat-a".to_owned(), None);
    let voice_scope = RiseMessageTabScope::new(RiseMessageTabKind::Voice, "chat-a".to_owned(), None);
    let file_snapshot = preparer.prepare_messages(account_id, file_scope.clone(), &page).await;
    let voice_snapshot = preparer.prepare_messages(account_id, voice_scope.clone(), &page).await;
    let presentation = activated_presentation(account_id, cx);

    cx.update(|cx| {
        presentation.update(cx, |presentation, cx| {
            presentation.publish_message_tab(file_snapshot, cx);
            presentation.publish_message_tab(voice_snapshot, cx);
        });
        let presentation = presentation.read(cx);
        let entry_message_ids = |scope: &RiseMessageTabScope| {
            presentation.message_tab_media_entries(scope).iter().map(|entry| entry.message_id.clone()).collect::<Vec<_>>()
        };

        assert_eq!(entry_message_ids(&file_scope), vec!["file-message".to_owned()]);
        assert_eq!(entry_message_ids(&voice_scope), vec!["voice-message".to_owned()]);
        assert_eq!(
            presentation.message_tab_media_entries(&voice_scope).first().map(|entry| entry.item.kind),
            Some(MessageMediaKind::Voice)
        );
        assert_eq!(presentation.message_tab_entry_index_by_id(&file_scope).get("file-message-0"), Some(&0));
        assert!(!presentation.message_tab_media_sections(&file_scope).is_empty());
        assert_eq!(presentation.message_tab_cursor(&file_scope), Some("next-message"));
        assert!(presentation.message_tab_has_more(&file_scope));
        assert!(presentation.message_tab_has_resolved_initial(&file_scope));
    });
    deactivate(&presentation, cx);
}

#[test]
fn same_revision_reuses_prepared_rows_but_publishes_new_load_metadata() {
    let message = tab_message_wire("gif-message", "animation", "gif", 1_750_000_000.0);
    let scope = RiseMessageTabScope::new(RiseMessageTabKind::Gif, "chat-a".to_owned(), None);
    let preparer = RiseMessageTabPresentationPreparer::new();
    let first = block_on(preparer.prepare_messages(
        "message-tab-account",
        scope.clone(),
        &auxiliary_page(vec![message], Some("first"), true, 12, RiseConversationAuxiliaryLoadState::Loading, true),
    ));
    let second = block_on(preparer.prepare_messages(
        "message-tab-account",
        scope,
        &auxiliary_page(Vec::new(), Some("second"), false, 12, RiseConversationAuxiliaryLoadState::Loaded, true),
    ));

    assert_eq!(
        first.content.media_entries.iter().map(|entry| entry.id.as_str()).collect::<Vec<_>>(),
        second.content.media_entries.iter().map(|entry| entry.id.as_str()).collect::<Vec<_>>()
    );
    assert_eq!(second.cursor.as_deref(), Some("second"));
    assert!(!second.has_more);
    assert_eq!(second.load_state, RiseConversationAuxiliaryLoadState::Loaded);
}

fn search_message_wire(id: &str, content_type: &str, media_type: &str) -> MessageWireDTO {
    decode(&format!(
        r#"{{
          "id": "{id}",
          "chat_id": "chat-search",
          "sender_id": "sender-search",
          "content_type": "{content_type}",
          "created_at": 1750000000,
          "media_items": [{{
            "file_id": "{id}-media",
            "file_url": "https://cdn.example/{id}",
            "media_type": "{media_type}"
          }}]
        }}"#
    ))
}

#[gpui::test]
async fn search_rows_are_prepared_off_main_and_published_per_query(cx: &mut TestAppContext) {
    let wire = search_message_wire("search-message", "image", "image");
    let scope = RiseGlobalMessageSearchScope::new(ChatSearchMessageKind::Media, "  Cats ").expect("the query is not empty");
    let other_scope = RiseGlobalMessageSearchScope::new(ChatSearchMessageKind::Media, "dogs").expect("the query is not empty");
    let preparer = RiseGlobalMessageSearchPresentationPreparer::new();
    let first = preparer
        .prepare(
            "global-search-account",
            &scope,
            &auxiliary_page(vec![wire], Some("first-cursor"), true, 21, RiseConversationAuxiliaryLoadState::Loaded, true),
        )
        .await;
    let metadata_only = preparer
        .prepare(
            "global-search-account",
            &scope,
            &auxiliary_page(Vec::new(), Some("second-cursor"), false, 21, RiseConversationAuxiliaryLoadState::Loaded, true),
        )
        .await;

    assert_eq!(scope.query, "cats");
    assert_eq!(
        first.results.iter().map(|result| result.id.as_str()).collect::<Vec<_>>(),
        metadata_only.results.iter().map(|result| result.id.as_str()).collect::<Vec<_>>()
    );
    assert_eq!(first.results.first().map(|result| result.position), Some(0));
    assert_eq!(
        first.results.first().and_then(|result| result.media_item.as_ref()).map(|item| item.kind),
        Some(MessageMediaKind::Image)
    );
    assert_eq!(metadata_only.cursor.as_deref(), Some("second-cursor"));

    let presentation = activated_presentation(&first.account_id, cx);
    cx.update(|cx| presentation.update(cx, |presentation, cx| presentation.publish_global_message_search(metadata_only, cx)));

    let (state, other_state) = cx.update(|cx| {
        let presentation = presentation.read(cx);
        (presentation.global_message_search_state(&scope), presentation.global_message_search_state(&other_scope))
    });
    assert_eq!(
        state.results.iter().map(|result| result.message.id.as_str()).collect::<Vec<_>>(),
        vec!["search-message"]
    );
    assert_eq!(state.cursor.as_deref(), Some("second-cursor"));
    assert!(!state.has_more);
    assert_eq!(state.status(), SaiWsSliceStatus::Fulfilled);
    assert!(other_state.results.is_empty());
    deactivate(&presentation, cx);
}

#[gpui::test]
async fn overlay_search_uses_typed_endpoints_and_publishes_prepared_pages(cx: &mut TestAppContext) {
    let global_endpoint =
        RiseConversationEndpoints::endpoint(rise_wire_methods::search::SERVICE, rise_wire_methods::search::GLOBAL_SEARCH)
            .expect("global search is a typed endpoint");
    let posts_endpoint =
        RiseConversationEndpoints::endpoint(rise_wire_methods::post::SERVICE, rise_wire_methods::post::SEARCH_POSTS)
            .expect("post search is a typed endpoint");
    assert_eq!(global_endpoint.kind, RiseConversationEndpointKind::Read);
    assert_eq!(posts_endpoint.kind, RiseConversationEndpointKind::Read);

    let entity: SearchResultItemDTO = decode(
        r#"{
          "entity_type": 0,
          "entity_id": "user-a",
          "user": {
            "id": "user-a",
            "name": "Alice",
            "tag": "alice"
          }
        }"#,
    );
    let chat: RiseStoredChat = decode(
        r#"{
          "id": "chat-a",
          "title": "Chat"
        }"#,
    );
    let post: GetFeedPostItemDTO = decode(
        r#"{
          "id": 42,
          "title": "Post"
        }"#,
    );
    let scope = RiseChatOverlaySearchScope::new("  Cats ").expect("the query is not empty");
    let preparer = RiseChatOverlaySearchPresentationPreparer::new();
    let entities = preparer
        .prepare_entities(
            "overlay-account",
            &scope,
            &auxiliary_page(vec![entity], Some("entity-cursor"), true, 1, RiseConversationAuxiliaryLoadState::Loaded, false),
        )
        .await;
    let chats = preparer
        .prepare_chats(
            "overlay-account",
            &scope,
            &auxiliary_page(vec![chat], Some("chat-cursor"), true, 2, RiseConversationAuxiliaryLoadState::Loaded, false),
        )
        .await;
    let posts = preparer
        .prepare_posts(
            "overlay-account",
            &scope,
            &auxiliary_page(vec![post], Some("post-cursor"), false, 3, RiseConversationAuxiliaryLoadState::Loaded, false),
        )
        .await;
    let presentation = activated_presentation("overlay-account", cx);
    cx.update(|cx| {
        presentation.update(cx, |presentation, cx| {
            presentation.publish_chat_overlay_entity_search(entities, cx);
            presentation.publish_chat_overlay_chat_search(chats, cx);
            presentation.publish_chat_overlay_post_search(posts, cx);
        });
    });

    let state = cx.update(|cx| presentation.read(cx).chat_overlay_search_state(&scope));
    assert_eq!(scope.query, "cats");
    assert_eq!(state.entities.items.iter().map(|item| item.id.as_str()).collect::<Vec<_>>(), vec!["user-a"]);
    assert_eq!(state.chats.items.iter().map(|item| item.id.as_str()).collect::<Vec<_>>(), vec!["chat-a"]);
    assert_eq!(state.posts.items.iter().map(|item| item.id.as_str()).collect::<Vec<_>>(), vec!["42"]);
    assert_eq!(state.entities.cursor.as_deref(), Some("entity-cursor"));
    assert_eq!(state.chats.cursor.as_deref(), Some("chat-cursor"));
    assert_eq!(state.posts.cursor.as_deref(), Some("post-cursor"));
    assert_eq!(state.posts.status(), SaiWsSliceStatus::Fulfilled);
    deactivate(&presentation, cx);
}

#[test]
fn search_generation_rejects_cancelled_stale_and_shutdown_results() {
    let first = RiseChatOverlaySearchScope::new("first").expect("the query is not empty");
    let second = RiseChatOverlaySearchScope::new("second").expect("the query is not empty");
    assert!(RiseSearchRequestGenerationPolicy::accepts(Some(&first), Some(7), &first, 7, false, false));
    assert!(!RiseSearchRequestGenerationPolicy::accepts(Some(&second), Some(8), &first, 7, false, false));
    assert!(!RiseSearchRequestGenerationPolicy::accepts(Some(&first), Some(7), &first, 7, true, false));
    assert!(!RiseSearchRequestGenerationPolicy::accepts(Some(&first), Some(7), &first, 7, false, true));
}
