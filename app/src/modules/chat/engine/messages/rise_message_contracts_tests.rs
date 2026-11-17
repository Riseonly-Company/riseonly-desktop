use std::collections::HashSet;

use rise_engine::RiseJSONValue;

use crate::modules::chat::engine::core::rise_conversation_auxiliary_repository::{
    RiseConversationAuxiliaryRepository, RiseConversationCompatibilityRequestCoordinator,
    RiseConversationCompatibilityRequestCoordinatorResource, RiseConversationCompatibilityRequestCoordinatorSignature,
};
use crate::modules::chat::engine::core::rise_conversation_resource_keys::RiseConversationResourceKeys;
use crate::modules::chat::engine::core::rise_conversation_resource_repository::{
    RiseConversationResourceKey, RiseConversationResourceKeyPersistence,
};
use crate::modules::chat::engine::messages::rise_message_contracts::{
    RiseMessageAuxiliaryInvalidation, RiseMessageAuxiliaryInvalidationPinnedMessagePatch,
    RiseMessageAuxiliaryProjection,
};

fn compatibility_key(
    method: &str,
    identity: &str,
    persistence: RiseConversationResourceKeyPersistence,
) -> RiseConversationResourceKey {
    RiseConversationResourceKey::with_persistence(["compatibility", "message", method, identity], persistence)
}

fn memory_companion(persistent: &RiseConversationResourceKey) -> RiseConversationResourceKey {
    RiseConversationResourceKey::with_persistence(
        ["compatibility-memory", persistent.key.as_str()],
        RiseConversationResourceKeyPersistence::MemoryOnly,
    )
}

#[test]
fn unpin_all_uses_explicit_projection_clear_patch() {
    let patch = RiseMessageAuxiliaryInvalidationPinnedMessagePatch::clear_all_pinned(true);
    assert!(patch.removes_all);
    assert!(!patch.is_pinned);
    assert!(patch.message_identifiers.is_empty());
    assert!(patch.message.is_none());
}

#[test]
fn pinned_invalidation_is_chat_scoped_and_does_not_fan_out_to_tabs() {
    let invalidation = RiseMessageAuxiliaryInvalidation::new(
        "chat-a",
        Some("topic-a"),
        HashSet::from([RiseMessageAuxiliaryProjection::Pinned]),
    );
    let keys = RiseConversationAuxiliaryRepository::message_projection_resource_keys(&invalidation);

    let typed = RiseConversationResourceKeys::pinned_messages("chat-a");
    let compatibility = compatibility_key(
        "get_pinned_messages",
        "getPinnedMessages-chat-a",
        RiseConversationResourceKeyPersistence::Persistent,
    );
    assert_eq!(keys.len(), 4);
    assert!(keys.contains(&typed));
    assert!(keys.contains(&compatibility));
    assert!(keys.contains(&memory_companion(&compatibility)));
    assert!(keys.contains(&compatibility_key(
        "get_pinned_messages",
        "getPinnedMessages-chat-a",
        RiseConversationResourceKeyPersistence::MemoryOnly,
    )));
    assert!(keys.iter().all(|key| key.key.contains("chat-a")));
    assert!(keys.iter().all(|key| !key.key.contains("topic-a")));

    let presentation_keys =
        RiseConversationAuxiliaryRepository::message_projection_presentation_cache_keys(&invalidation, "account-a");
    assert_eq!(presentation_keys, HashSet::from(["account:account-a:getPinnedMessages-chat-a".to_owned()]));
}

#[test]
fn content_invalidation_targets_root_and_exact_topic_only() {
    let projections = HashSet::from([
        RiseMessageAuxiliaryProjection::Media,
        RiseMessageAuxiliaryProjection::Files,
        RiseMessageAuxiliaryProjection::Favorites,
        RiseMessageAuxiliaryProjection::Links,
    ]);
    let invalidation = RiseMessageAuxiliaryInvalidation::new("chat-a", Some("topic-a"), projections.clone());
    let keys = RiseConversationAuxiliaryRepository::message_projection_resource_keys(&invalidation);

    assert_eq!(keys.len(), projections.len() * 8);
    assert!(keys.contains(&RiseConversationResourceKeys::message_tab("media", "chat-a", None)));
    assert!(keys.contains(&RiseConversationResourceKeys::message_tab("media", "chat-a", Some("topic-a"))));
    assert!(keys.contains(&compatibility_key(
        "get_media_messages",
        "getMediaMessages-chat-a::t::topic-a",
        RiseConversationResourceKeyPersistence::Persistent,
    )));
    assert!(keys.contains(&compatibility_key(
        "get_fav_messages",
        "get_fav_messages-chat-a",
        RiseConversationResourceKeyPersistence::Persistent,
    )));
    assert!(keys.iter().all(|key| key.key.contains("chat-a")));
    assert!(keys.iter().all(|key| !key.key.contains("chat-b")));
    assert!(!keys.contains(&RiseConversationResourceKeys::pinned_messages("chat-a")));
    assert!(!keys.contains(&RiseConversationResourceKeys::message_tab("music", "chat-a", None)));

    let presentation_keys =
        RiseConversationAuxiliaryRepository::message_projection_presentation_cache_keys(&invalidation, "account-a");
    assert_eq!(presentation_keys.len(), projections.len() * 2);
    assert!(presentation_keys.contains("account:account-a:getMediaMessages-chat-a::t::topic-a"));
    assert!(presentation_keys.iter().all(|key| key.starts_with("account:account-a:") && !key.contains("chat-b")));
}

#[tokio::test]
async fn targeted_compatibility_invalidation_leaves_other_resource_current() -> anyhow::Result<()> {
    let coordinator = RiseConversationCompatibilityRequestCoordinator::new();
    let first = RiseConversationCompatibilityRequestCoordinatorResource {
        account_id: "account-a".to_owned(),
        logical_key: "chat-a".to_owned(),
    };
    let second = RiseConversationCompatibilityRequestCoordinatorResource {
        account_id: "account-b".to_owned(),
        logical_key: "chat-b".to_owned(),
    };
    let signature = RiseConversationCompatibilityRequestCoordinatorSignature {
        body_digest: "body".to_owned(),
        prefer_cached_value: false,
        persistence: RiseConversationResourceKeyPersistence::Persistent,
        allow_persistent_creation: true,
    };
    let first_output = coordinator
        .perform_read(first.clone(), signature.clone(), || async { Ok(RiseJSONValue::Integer(1)) })
        .await?;
    let second_output =
        coordinator.perform_read(second, signature, || async { Ok(RiseJSONValue::Integer(2)) }).await?;

    coordinator.invalidate_resources(&HashSet::from([first]));

    assert!(!coordinator.is_current(&first_output.receipt).await);
    assert!(coordinator.is_current(&second_output.receipt).await);
    Ok(())
}
