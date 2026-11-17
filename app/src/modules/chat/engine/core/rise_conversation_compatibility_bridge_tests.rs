use std::collections::{HashMap, HashSet};

use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::SaiWsDiskCachePolicy;
use crate::core::lib::sai::modules::sai_ws::sai_ws_screen_refresh::SaiActiveScreen;
use crate::modules::chat::engine::core::rise_conversation_auxiliary_repository::RiseConversationResolutionSource;
use crate::modules::chat::engine::core::rise_conversation_compatibility_bridge::{
    RiseConversationCompatibilityBridge, RiseConversationCompatibilityBridgeScreenRefreshDisposition,
};
use crate::modules::chat::engine::core::rise_conversation_presentation::RiseChatListKind;
use crate::modules::chat::engine::core::rise_conversation_resource_repository::RiseConversationResourceKeyPersistence;
use crate::modules::chat::engine::rise_conversation_domain::RiseConversationDomain;

fn screen(id: &str, params: &[(&str, &str)]) -> SaiActiveScreen {
    SaiActiveScreen::new(
        id.to_owned(),
        params.iter().map(|(key, value)| ((*key).to_owned(), (*value).to_owned())).collect::<HashMap<_, _>>(),
    )
}

fn kinds(id: &str, params: &[(&str, &str)]) -> HashSet<RiseChatListKind> {
    RiseConversationDomain::chat_kinds_to_synchronize(Some(&screen(id, params)))
}

#[test]
fn pagination_caches_only_the_merged_logical_root() {
    let plan = RiseConversationCompatibilityBridge::cache_plan(SaiWsDiskCachePolicy::Persistent, true, true);

    assert_eq!(plan.persistence, RiseConversationResourceKeyPersistence::Persistent);
    assert!(plan.allow_persistent_creation);
    assert!(!plan.cache_wire_response);
    assert!(plan.cache_merged_response);
}

#[test]
fn non_caching_request_never_writes_wire_or_merged_response() {
    for is_paginated in [false, true] {
        let plan = RiseConversationCompatibilityBridge::cache_plan(SaiWsDiskCachePolicy::Persistent, false, is_paginated);

        assert!(!plan.cache_wire_response);
        assert!(!plan.cache_merged_response);
    }
}

#[test]
fn preserve_existing_cannot_create_persistent_resource() {
    let plan = RiseConversationCompatibilityBridge::cache_plan(SaiWsDiskCachePolicy::PreserveExisting, true, false);

    assert_eq!(plan.persistence, RiseConversationResourceKeyPersistence::Persistent);
    assert!(!plan.allow_persistent_creation);
    assert!(plan.cache_wire_response);
    assert!(!plan.cache_merged_response);
}

#[test]
fn preserve_existing_pagination_still_defers_until_merge() {
    let plan = RiseConversationCompatibilityBridge::cache_plan(SaiWsDiskCachePolicy::PreserveExisting, true, true);

    assert!(!plan.allow_persistent_creation);
    assert!(!plan.cache_wire_response);
    assert!(plan.cache_merged_response);
}

#[test]
fn memory_only_never_selects_persistent_storage() {
    let plan = RiseConversationCompatibilityBridge::cache_plan(SaiWsDiskCachePolicy::MemoryOnly, true, false);

    assert_eq!(plan.persistence, RiseConversationResourceKeyPersistence::MemoryOnly);
    assert!(plan.allow_persistent_creation);
    assert!(plan.cache_wire_response);
}

#[test]
fn cached_compatibility_read_schedules_only_one_cycle_revalidation() {
    assert_eq!(
        RiseConversationCompatibilityBridge::screen_refresh_disposition(Some(
            RiseConversationResolutionSource::ResourceMemory
        )),
        RiseConversationCompatibilityBridgeScreenRefreshDisposition::RevalidateCachedValue
    );
    assert_eq!(
        RiseConversationCompatibilityBridge::screen_refresh_disposition(Some(
            RiseConversationResolutionSource::PersistentStore
        )),
        RiseConversationCompatibilityBridgeScreenRefreshDisposition::RevalidateCachedValue
    );
}

#[test]
fn completed_network_read_closes_the_current_revalidation_cycle() {
    assert_eq!(
        RiseConversationCompatibilityBridge::screen_refresh_disposition(Some(RiseConversationResolutionSource::Network)),
        RiseConversationCompatibilityBridgeScreenRefreshDisposition::MarkNetworkResolved
    );
    assert_eq!(
        RiseConversationCompatibilityBridge::screen_refresh_disposition(Some(
            RiseConversationResolutionSource::NetworkFallbackPersistentStore
        )),
        RiseConversationCompatibilityBridgeScreenRefreshDisposition::MarkNetworkResolved
    );
}

#[test]
fn foreground_chat_synchronization_targets_only_the_visible_list() {
    assert_eq!(kinds("Chats", &[]), HashSet::from([RiseChatListKind::Inbox]));
    assert_eq!(kinds("ArchivedChats", &[]), HashSet::from([RiseChatListKind::Archive]));
    assert_eq!(kinds("MessageRequests", &[]), HashSet::from([RiseChatListKind::Requests]));
}

#[test]
fn foreground_forum_synchronizes_inbox_but_ordinary_chat_does_not() {
    assert_eq!(
        kinds("Chat", &[("chatId", "chat-1"), ("topicMode", "all")]),
        HashSet::from([RiseChatListKind::Inbox])
    );
    assert_eq!(kinds("ForumHome", &[("chatId", "chat-1")]), HashSet::from([RiseChatListKind::Inbox]));
    assert!(kinds("Chat", &[("chatId", "chat-1")]).is_empty());
}
