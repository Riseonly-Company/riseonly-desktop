use std::collections::HashSet;

use rise_canon::rise_request;
use serde_json::Value;

use crate::core::lib::sai::core::utilities::sai_payload_path::SaiJSONCodec;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiWsSliceStatus;
use crate::modules::chat::components::chat_search::chat_search_overlay::ChatSearchHero;
use crate::modules::post::stores::post_actions::post_types::AuthorInfo;
use crate::modules::user::components::user_relationship_list_contract::{
    UserRelationshipRowMetrics, UserRelationshipSearchHero,
};
use crate::modules::user::stores::user::user_actions::user_types::{
    FollowButtonState, UserListContinuation, UserListEntry, UserListKind, UserRelationshipFailureCacheContract,
    UserRelationshipListCacheRegistryContract, UserRelationshipListCacheScope, UserRelationshipListRenderContract,
    UserRelationshipSearchContract,
};
use crate::modules::user::stores::user::user_actions::user_wire_types::{
    FriendListGatewayDTO, FriendRequestGatewayDTO, UserSubsListGatewayDTO,
};

fn parsed(raw: &str) -> Value {
    match serde_json::from_str(raw) {
        Ok(value) => value,
        Err(error) => panic!("fixture is not JSON: {error}"),
    }
}

#[derive(Default)]
struct EntryFixture {
    author_id: Option<&'static str>,
    tag: Option<&'static str>,
    created_at: Option<&'static str>,
    entry_key: Option<&'static str>,
    owner_is_friend: bool,
    viewer_is_subbed: Option<bool>,
    viewer_is_subscriber: Option<bool>,
    viewer_is_friend: Option<bool>,
}

fn entry(fixture: EntryFixture) -> UserListEntry {
    UserListEntry {
        entry_key: fixture.entry_key.map(str::to_owned),
        entry_id: None,
        author: AuthorInfo {
            id: fixture.author_id.map(str::to_owned),
            name: "Name".to_owned(),
            tag: fixture.tag.unwrap_or("tag").to_owned(),
            ..AuthorInfo::default()
        },
        owner_is_friend: fixture.owner_is_friend,
        viewer_is_subbed: fixture.viewer_is_subbed,
        viewer_is_subscriber: fixture.viewer_is_subscriber,
        viewer_is_friend: fixture.viewer_is_friend,
        viewer_friend_request_id: None,
        created_at: fixture.created_at.map(str::to_owned),
    }
}

#[test]
fn rows_match_chat_list_geometry_and_accessible_action_size() {
    assert_eq!(UserRelationshipRowMetrics::AVATAR_SIZE, 50.0);
    assert_eq!(UserRelationshipRowMetrics::CONTENT_SPACING, 12.0);
    assert_eq!(UserRelationshipRowMetrics::HORIZONTAL_PADDING, 12.0);
    assert_eq!(UserRelationshipRowMetrics::VERTICAL_PADDING, 10.0);
    assert!(UserRelationshipRowMetrics::relationship_button_minimum_height() >= 44.0);
}

#[test]
fn relationship_search_uses_dedicated_hero_identity() {
    assert!(!UserRelationshipSearchHero::FIELD_ID.is_empty());
    assert_ne!(UserRelationshipSearchHero::FIELD_ID, ChatSearchHero::FIELD_ID);
}

#[test]
fn relationship_search_caps_and_normalizes_cache_identity() {
    let oversized = "A".repeat(180);
    assert_eq!(UserRelationshipSearchContract::limited_input(&oversized).chars().count(), 128);
    let unicode = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}\u{200D}\u{1F466}".repeat(40);
    assert_eq!(UserRelationshipSearchContract::limited_input(&unicode).chars().count(), 128);
    let cache_query = |value: &str| UserRelationshipSearchContract::cache_query(Some(value));
    assert_eq!(UserRelationshipSearchContract::outbound_query(Some("  Alice  ")).as_deref(), Some("Alice"));
    assert_eq!(UserRelationshipSearchContract::outbound_query(Some(" @Alice ")).as_deref(), Some("@Alice"));
    assert_eq!(cache_query(" @Alice ").as_deref(), Some("tag:alice"));
    assert_eq!(cache_query("alice").as_deref(), Some("name:alice"));
    assert_eq!(cache_query("@").as_deref(), Some("tag:"));
    assert_eq!(cache_query("@@").as_deref(), Some("tag:@"));
    assert_eq!(cache_query("@@alice").as_deref(), Some("tag:@alice"));
    assert_ne!(cache_query("@alice"), cache_query("alice"));
    assert_ne!(cache_query("@"), cache_query("@@"));
}

#[test]
fn short_relationship_search_never_dispatches() {
    let is_short = |value: &str| UserRelationshipSearchContract::is_short_nonempty_query(Some(value));
    let dispatches = |value: &str| UserRelationshipSearchContract::should_dispatch(Some(value));
    assert_eq!(UserRelationshipSearchContract::minimum_characters(), 3);
    assert_eq!(UserRelationshipSearchContract::normalized_term(Some("@alice")).as_deref(), Some("alice"));
    assert_eq!(UserRelationshipSearchContract::normalized_term(Some("@@alice")).as_deref(), Some("@alice"));
    assert!(is_short("a"));
    assert!(is_short("@ab"));
    assert!(is_short("@"));
    assert!(is_short("@@"));
    assert!(!dispatches("ab"));
    assert!(dispatches("abc"));
    assert!(dispatches("@abc"));
    assert!(dispatches("@@alice"));
    assert!(dispatches(""));
}

#[test]
fn entry_identity_never_falls_back_to_random_uuid() {
    let author_entry = entry(EntryFixture { author_id: Some("user-42"), ..EntryFixture::default() });
    assert_eq!(author_entry.id(), "user:user-42");
    assert_eq!(author_entry.id(), author_entry.id());

    let anonymous = entry(EntryFixture { tag: Some("same"), created_at: Some("2026-07-15"), ..EntryFixture::default() });
    assert_eq!(anonymous.id(), "anonymous:same|name|2026-07-15");
    assert_eq!(anonymous.id(), anonymous.id());

    let keyed = entry(EntryFixture {
        author_id: Some("user-42"),
        entry_key: Some("followers:91"),
        ..EntryFixture::default()
    });
    assert_eq!(keyed.id(), "entry:followers:91");
}

#[test]
fn viewer_relationship_overrides_owner_list_semantics() {
    let explicit = entry(EntryFixture {
        author_id: Some("user-42"),
        owner_is_friend: true,
        viewer_is_subbed: Some(false),
        viewer_is_subscriber: Some(true),
        viewer_is_friend: Some(false),
        ..EntryFixture::default()
    });
    let relationship = explicit.resolved_relationship(UserListKind::Subscriptions, false);

    assert_eq!(relationship.follow_button_state(), FollowButtonState::SubscribeBack);
}

#[test]
fn own_lists_have_safe_legacy_fallbacks_only_for_their_owner() {
    let friendship = entry(EntryFixture { author_id: Some("user-42"), owner_is_friend: true, ..EntryFixture::default() });
    assert_eq!(
        friendship.resolved_relationship(UserListKind::Friends, true).follow_button_state(),
        FollowButtonState::Friends
    );
    assert_eq!(
        friendship.resolved_relationship(UserListKind::Friends, false).follow_button_state(),
        FollowButtonState::Subscribe
    );

    let subscription = entry(EntryFixture { author_id: Some("user-42"), ..EntryFixture::default() });
    assert_eq!(
        subscription.resolved_relationship(UserListKind::Subscriptions, true).follow_button_state(),
        FollowButtonState::Following
    );
    assert_eq!(
        subscription.resolved_relationship(UserListKind::Subscriptions, false).follow_button_state(),
        FollowButtonState::Subscribe
    );
}

#[test]
fn scoped_render_status_does_not_leak_another_query_slice() {
    let status = UserRelationshipListRenderContract::status;
    assert_eq!(status(false, false, false, SaiWsSliceStatus::Fulfilled), SaiWsSliceStatus::Idle);
    assert_eq!(status(false, false, false, SaiWsSliceStatus::Rejected), SaiWsSliceStatus::Idle);
    assert_eq!(status(false, false, true, SaiWsSliceStatus::Fulfilled), SaiWsSliceStatus::Rejected);
    assert_eq!(status(false, true, true, SaiWsSliceStatus::Pending), SaiWsSliceStatus::Pending);
    assert_eq!(status(true, false, true, SaiWsSliceStatus::Rejected), SaiWsSliceStatus::Fulfilled);
}

#[test]
fn failed_query_cache_is_memory_bounded() {
    let maximum = UserRelationshipFailureCacheContract::maximum_entries();
    let mut cache_keys: HashSet<String> = HashSet::new();
    for index in 0..=maximum {
        UserRelationshipFailureCacheContract::record(&format!("query-{index}"), &mut cache_keys);
    }

    assert!(cache_keys.len() as i64 <= maximum);
    assert!(cache_keys.contains(&format!("query-{maximum}")));
}

#[test]
fn successful_query_cache_registry_is_bounded_and_uses_recent_scopes() {
    let maximum = UserRelationshipListCacheRegistryContract::maximum_entries();
    let mut scopes: Vec<UserRelationshipListCacheScope> = Vec::new();
    let mut evicted: Vec<UserRelationshipListCacheScope> = Vec::new();
    for index in 0..=maximum {
        let search = format!("query-{index}");
        let scope = UserRelationshipListCacheScope::new(UserListKind::Subscriptions, "viewer".to_owned(), Some(&search));
        evicted.extend(UserRelationshipListCacheRegistryContract::record(&scope, &mut scopes));
    }

    assert_eq!(scopes.len() as i64, maximum);
    assert_eq!(evicted.len(), 1);
    assert_eq!(evicted.first().and_then(|scope| scope.query.as_deref()), Some("name:query-0"));
    let last_query = format!("name:query-{maximum}");
    assert_eq!(scopes.last().and_then(|scope| scope.query.as_deref()), Some(last_query.as_str()));

    let recently_used = scopes[3].clone();
    let _ = UserRelationshipListCacheRegistryContract::record(&recently_used, &mut scopes);
    assert_eq!(scopes.last(), Some(&recently_used));
    assert_eq!(scopes.len() as i64, maximum);
}

#[test]
fn follow_mutation_patches_every_registered_query_scope() {
    let viewer_id = "viewer";
    let target_id = "target";
    let scope = |kind, owner: &str, search| UserRelationshipListCacheScope::new(kind, owner.to_owned(), search);
    let scopes = [
        scope(UserListKind::Subscriptions, viewer_id, None),
        scope(UserListKind::Subscriptions, viewer_id, Some("alice")),
        scope(UserListKind::Friends, viewer_id, Some("bob")),
        scope(UserListKind::Subscribers, target_id, Some("charlie")),
        scope(UserListKind::Friends, target_id, None),
        scope(UserListKind::Subscribers, viewer_id, Some("alice")),
        scope(UserListKind::Subscriptions, target_id, Some("alice")),
        scope(UserListKind::Friends, "someone-else", Some("alice")),
    ];

    assert!(scopes.iter().all(|scope| {
        UserRelationshipListCacheRegistryContract::is_affected_by_follow_mutation(scope, viewer_id, target_id)
    }));
}

#[test]
fn follow_mutation_response_carries_authoritative_relationship() {
    let payload = parsed(r#"{"success": true, "is_subbed": true, "is_subscriber": true, "is_friend": true}"#);
    let response = SaiJSONCodec::decode::<FriendRequestGatewayDTO>(Some(&payload));

    assert!(response.is_some());
}

#[test]
fn opaque_and_legacy_pagination_tokens_remain_typed() {
    assert_eq!(
        UserListContinuation::resolve(Some(" opaque-cursor "), Some(42)),
        Some(UserListContinuation::Cursor("opaque-cursor".to_owned()))
    );
    assert_eq!(UserListContinuation::resolve(None, Some(42)), Some(UserListContinuation::LegacyRelativeId(42)));
    assert_eq!(UserListContinuation::resolve(Some("  "), None), None);
}

#[test]
fn wire_contract_carries_cursor_and_entry_identity_separately() {
    let body = rise_request::user::GetUserFriends {
        user_id: "owner".to_owned(),
        cursor: Some("opaque-cursor".to_owned()),
        limit: Some(20),
        relative_id: None,
        search: Some("alice".to_owned()),
        up: Some(true),
    };
    let encoded = SaiJSONCodec::encode(&body);
    assert_eq!(encoded.as_ref().and_then(|map| map.get("cursor")).and_then(Value::as_str), Some("opaque-cursor"));
    assert!(encoded.as_ref().and_then(|map| map.get("relative_id")).is_none());

    let friends = parsed(
        r#"{"items": [{"entry_key": "friend:91", "id": 91, "user": {"id": "user-42"}}], "has_more": true, "cursor": "next-opaque-cursor"}"#,
    );
    let decoded = SaiJSONCodec::decode::<FriendListGatewayDTO>(Some(&friends));
    assert_eq!(decoded.as_ref().and_then(|dto| dto.cursor.as_deref()), Some("next-opaque-cursor"));
    assert_eq!(
        decoded.as_ref().and_then(|dto| dto.items.first()).map(|item| item.entry_key.as_str()),
        Some("friend:91")
    );

    let subs = parsed(r#"{"items": [], "has_more": true, "relative_id": "42"}"#);
    let legacy_string = SaiJSONCodec::decode::<UserSubsListGatewayDTO>(Some(&subs));
    assert_eq!(legacy_string.and_then(|dto| dto.relative_id), Some(42));
}
