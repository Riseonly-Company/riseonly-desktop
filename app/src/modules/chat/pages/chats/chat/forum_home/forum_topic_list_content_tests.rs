use std::cell::RefCell;

#[cfg(any(debug_assertions, feature = "sai_debug_tools"))]
use crate::core::lib::sai::modules::sai_debugger::sai_performance_monitor::SaiPerformanceSamplingPolicy;
use crate::modules::chat::pages::chats::chat::forum_home::forum_topic_list_content::{
    ForumHomeColdPresentationPhase, ForumHomeColdPresentationPolicy, ForumHomeInitialRefreshPolicy,
    ForumVoiceObservationLifecycle, changed_row_span,
};
use crate::modules::chat::pages::chats::embedded_forum_transition::EmbeddedForumTransitionMetrics;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{
    ChatInfo, ChatMuteScopes, ChatPerm, ChatTopic,
};
use crate::modules::chat::stores::chats::chat_services::chat_roles_services::{
    ForumTopicGrouping, ForumTopicListSnapshot,
};

fn update(observation: &mut ForumVoiceObservationLifecycle, chat_id: &str, signature: &str, accepts: bool, calls: &RefCell<Vec<String>>) {
    observation.update(
        chat_id,
        signature,
        accepts,
        |id| calls.borrow_mut().push(format!("begin:{id}")),
        |id| calls.borrow_mut().push(format!("end:{id}")),
    );
}

fn stop(observation: &mut ForumVoiceObservationLifecycle, calls: &RefCell<Vec<String>>) {
    observation.stop(|id| calls.borrow_mut().push(format!("end:{id}")));
}

fn topic(id: &str, position: i64, is_general: bool, section_id: &str) -> ChatTopic {
    let mut topic =
        ChatTopic::new(id.to_owned(), id.to_owned(), ChatTopic::DEFAULT_ICON.to_owned(), position, is_general, false);
    topic.section_id = section_id.to_owned();
    topic
}

fn ids(topics: &[ChatTopic]) -> Vec<&str> {
    topics.iter().map(|topic| topic.id.as_str()).collect()
}

#[test]
fn quick_close_does_not_release_an_observation_that_never_started() {
    let mut observation = ForumVoiceObservationLifecycle::new();
    let calls = RefCell::new(Vec::new());
    update(&mut observation, "forum", "voice", false, &calls);
    stop(&mut observation, &calls);
    assert!(calls.borrow().is_empty());

    update(&mut observation, "forum", "", true, &calls);
    stop(&mut observation, &calls);
    assert!(calls.borrow().is_empty(), "A text-only forum has no voice observation to release");
}

#[test]
fn voice_observation_updates_its_channels_and_releases_once() {
    let mut observation = ForumVoiceObservationLifecycle::new();
    let calls = RefCell::new(Vec::new());
    for signature in ["voice-a", "voice-a", "voice-a,voice-b"] {
        update(&mut observation, "forum", signature, true, &calls);
    }
    assert_eq!(*calls.borrow(), ["begin:forum", "begin:forum"], "Roster changes must not clear live presence");
    stop(&mut observation, &calls);
    stop(&mut observation, &calls);
    assert_eq!(*calls.borrow(), ["begin:forum", "begin:forum", "end:forum"]);
}

#[test]
fn changing_forum_or_removing_last_voice_channel_balances_observation() {
    let mut observation = ForumVoiceObservationLifecycle::new();
    let calls = RefCell::new(Vec::new());
    for (chat_id, signature) in [("a", "voice"), ("b", "voice"), ("b", "")] {
        update(&mut observation, chat_id, signature, true, &calls);
    }
    stop(&mut observation, &calls);
    assert_eq!(*calls.borrow(), ["begin:a", "end:a", "begin:b", "end:b"]);
}

#[cfg(any(debug_assertions, feature = "sai_debug_tools"))]
#[test]
fn a_small_hitch_cannot_hide_the_next_large_stall() {
    assert!(!SaiPerformanceSamplingPolicy::should_record_hitch(25.0, 1.02, 1.0));
    assert!(SaiPerformanceSamplingPolicy::should_record_hitch(136.39, 1.10, 1.0));
    assert!(SaiPerformanceSamplingPolicy::should_record_hitch(61.69, 1.12, 1.10));
    assert!(SaiPerformanceSamplingPolicy::should_record_hitch(25.0, 1.25, 1.0));
}

#[test]
fn grouping_partitions_topics_once_and_preserves_presentation_order() {
    let topics = [
        topic("general", 0, true, ""),
        topic("free-1", 1, false, ""),
        topic("section-a-1", 2, false, "a"),
        topic("section-b-1", 3, false, "b"),
        topic("section-a-2", 4, false, "a"),
        topic("free-2", 5, false, ""),
    ];

    let grouping = ForumTopicGrouping::new(&topics);

    assert_eq!(ids(&grouping.uncategorized), ["free-1", "free-2"]);
    assert_eq!(ids(grouping.topics("a")), ["section-a-1", "section-a-2"]);
    assert_eq!(ids(grouping.topics("b")), ["section-b-1"]);
    assert!(grouping.topics("missing").is_empty());
}

#[test]
fn snapshot_resolves_permissions_and_mute_without_store_lookups_per_row() {
    let inherited_topic = topic("inherited", 0, false, "");
    let mut independent_topic = topic("independent", 1, false, "");
    independent_topic.notification_inherited = false;
    independent_topic.notification_scopes = ChatMuteScopes::ALL_ENABLED;

    let snapshot = ForumTopicListSnapshot {
        chat: ChatInfo::new("chat".to_owned()),
        sections: Vec::new(),
        general_topic: None,
        grouping: ForumTopicGrouping::new(&[inherited_topic.clone(), independent_topic.clone()]),
        permissions: ChatPerm::MANAGE_TOPICS | ChatPerm::MANAGE_ROLES,
        inherited_notification_scopes: ChatMuteScopes::MUTED,
        global_mute_forces_muted: false,
        has_cached_structure: true,
        general_hidden: false,
    };

    assert!(snapshot.can_manage_topics());
    assert!(!snapshot.can_manage_voice_channels());
    assert!(snapshot.can_create_channels());
    assert!(!snapshot.can_manage_sections());
    assert!(snapshot.can_manage_roles());
    assert!(snapshot.is_effectively_muted(&inherited_topic));
    assert!(!snapshot.is_effectively_muted(&independent_topic));

    let globally_muted = ForumTopicListSnapshot {
        chat: snapshot.chat.clone(),
        sections: Vec::new(),
        general_topic: None,
        grouping: snapshot.grouping.clone(),
        permissions: 0,
        inherited_notification_scopes: ChatMuteScopes::ALL_ENABLED,
        global_mute_forces_muted: true,
        has_cached_structure: true,
        general_hidden: false,
    };
    assert!(globally_muted.is_effectively_muted(&independent_topic));
}

#[test]
fn every_initial_refresh_protects_opening_and_first_close_gesture() {
    assert!(ForumHomeInitialRefreshPolicy::delay_nanoseconds(false) >= 800_000_000);
    assert!(ForumHomeInitialRefreshPolicy::delay_nanoseconds(true) >= 800_000_000);
    let retry = ForumHomeInitialRefreshPolicy::INTERACTION_RETRY_DELAY_NANOSECONDS;
    assert!(retry >= 80_000_000, "Deferred refresh must not poll the main actor every display frame");
    let (refresh, work) = (
        EmbeddedForumTransitionMetrics::BACKGROUND_REFRESH_DELAY_DURATION,
        EmbeddedForumTransitionMetrics::BACKGROUND_WORK_DELAY_DURATION,
    );
    assert!(refresh > work, "Cache hydration and live projection must not share one main turn");
}

#[test]
fn cold_presentation_uses_bounded_stable_loading_and_recovers_late() {
    assert_eq!(ForumHomeColdPresentationPolicy::phase(false, false), ForumHomeColdPresentationPhase::Loading);
    assert_eq!(ForumHomeColdPresentationPolicy::phase(false, true), ForumHomeColdPresentationPhase::TimedOut);
    assert_eq!(
        ForumHomeColdPresentationPolicy::phase(true, true),
        ForumHomeColdPresentationPhase::Content,
        "A late response must recover automatically after timeout"
    );
    let timeout = ForumHomeColdPresentationPolicy::TIMEOUT_NANOSECONDS;
    assert!(timeout >= 5_000_000_000);
    assert!(timeout <= 15_000_000_000);
    let placeholder_row_count = ForumHomeColdPresentationPolicy::PLACEHOLDER_ROW_COUNT;
    assert!(placeholder_row_count >= 4);
}

fn row_keys(keys: &[&str]) -> Vec<String> {
    keys.iter().map(|key| (*key).to_owned()).collect()
}

#[test]
fn a_changed_row_splices_only_between_the_common_ends() {
    let old = row_keys(&["top", "all", "topic-1", "topic-2", "bottom"]);

    let inserted = row_keys(&["top", "all", "topic-1", "topic-3", "topic-2", "bottom"]);
    assert_eq!(changed_row_span(&old, &inserted), (3..3, 1));

    let removed = row_keys(&["top", "all", "topic-2", "bottom"]);
    assert_eq!(changed_row_span(&old, &removed), (2..3, 0));

    assert_eq!(changed_row_span(&old, &old), (5..5, 0));
    assert_eq!(changed_row_span(&[], &old), (0..0, 5));
}

#[test]
fn equal_neighbours_never_count_twice() {
    let old = row_keys(&["footer", "footer"]);
    let new = row_keys(&["footer"]);
    assert_eq!(changed_row_span(&old, &new), (1..2, 0));
    assert_eq!(changed_row_span(&new, &old), (1..1, 1));
}
