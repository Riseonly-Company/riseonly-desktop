use crate::app::router::navigation_types::RootRoute;
use crate::modules::search::shared::global_search_route_resolver::GlobalSearchRouteResolver;
use crate::modules::search::stores::search_actions::search_types::SearchEntityRow;

#[test]
fn user_result_with_private_chat_id_opens_user_profile() {
    let row = SearchEntityRow {
        id: "user-7".to_owned(),
        entity_type: "USER".to_owned(),
        display_name: "Twink".to_owned(),
        tag_line: "@twink".to_owned(),
        user_id: Some("user-7".to_owned()),
        user_tag: Some("twink".to_owned()),
        chat_id: Some("private-chat-42".to_owned()),
        ..SearchEntityRow::default()
    };

    assert_eq!(
        GlobalSearchRouteResolver::route(&row),
        Some(RootRoute::UserPage { user_id: Some("user-7".to_owned()), tag: Some("twink".to_owned()) })
    );
}

#[test]
fn user_result_without_nested_user_id_uses_entity_id_instead_of_private_chat() {
    let row = SearchEntityRow {
        id: "user-8".to_owned(),
        entity_type: "USER".to_owned(),
        display_name: "Incomplete User".to_owned(),
        chat_id: Some("private-chat-99".to_owned()),
        ..SearchEntityRow::default()
    };

    assert_eq!(
        GlobalSearchRouteResolver::route(&row),
        Some(RootRoute::UserPage { user_id: Some("user-8".to_owned()), tag: None })
    );
}

#[test]
fn channel_result_opens_channel_chat() {
    let row = SearchEntityRow {
        id: "channel-3".to_owned(),
        entity_type: "CHANNEL".to_owned(),
        display_name: "News".to_owned(),
        tag_line: "@news".to_owned(),
        chat_id: Some("channel-chat-3".to_owned()),
        is_official: true,
        ..SearchEntityRow::default()
    };

    assert_eq!(
        GlobalSearchRouteResolver::route(&row),
        Some(RootRoute::Chat { chat_id: Some("channel-chat-3".to_owned()) })
    );
}
