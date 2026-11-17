use serde::de::DeserializeOwned;

use crate::modules::music::stores::music_actions::music_wire_types::WireSearchDTO;
use crate::modules::search::stores::search_actions::search_wire_types::{GlobalSearchGatewayDTO, SearchUsersListDTO};
use crate::modules::search::stores::search_services::search_services::SearchServicesStore;

enum SearchPaginationDecodeSupport {}

impl SearchPaginationDecodeSupport {
    fn decode<T: DeserializeOwned>(json: &str) -> T {
        serde_json::from_str(json).expect("the fixture decodes")
    }
}

#[test]
fn global_search_chat_lane_decodes_independently() {
    let json = r#"
    {
      "query": "chat",
      "chats": [{ "id": "chat-1", "title": "Chat" }],
      "chats_has_more": true,
      "chats_next_cursor": "chat-1",
      "global_results": [],
      "global_has_more": false
    }
    "#;

    let dto: GlobalSearchGatewayDTO = SearchPaginationDecodeSupport::decode(json);

    assert_eq!(dto.chats.iter().map(|chat| chat.id.as_str()).collect::<Vec<_>>(), ["chat-1"]);
    assert!(dto.chats_has_more);
    assert_eq!(dto.chats_next_cursor.as_deref(), Some("chat-1"));
    assert!(!dto.global_has_more);
}

#[test]
fn global_and_chat_search_use_different_sai_lanes() {
    let global = SearchServicesStore::global_search_cache_key("same", "user-1");
    let chats = SearchServicesStore::global_search_chats_cache_key("same", "user-1");

    assert_ne!(global, chats);
}

#[test]
fn music_search_decodes_stable_continuation_metadata() {
    let json = r#"
    {
      "tracks": [],
      "nextCursor": "1700000000000:track-2",
      "hasMore": true
    }
    "#;

    let dto: WireSearchDTO = SearchPaginationDecodeSupport::decode(json);

    assert_eq!(dto.next_cursor.as_deref(), Some("1700000000000:track-2"));
    assert!(dto.has_more);
}

#[test]
fn search_users_decodes_backend_root_without_take_path() {
    let json = r#"
    {
      "list": [{ "id": "user-1", "name": "User" }],
      "is_have_more": true,
      "relative_id": 17
    }
    "#;

    let dto: SearchUsersListDTO = SearchPaginationDecodeSupport::decode(json);

    assert_eq!(dto.list.first().map(|user| user.id.as_str()), Some("user-1"));
    assert!(dto.is_have_more);
    assert_eq!(dto.relative_id, Some(17));
}
