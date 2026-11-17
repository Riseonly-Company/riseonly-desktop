use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::foundation_url::Url;

use crate::core::lib::sai::core::websocket::sai_web_socket_manager::SaiWebSocketManagerPhase;
use crate::modules::chat::pages::chats::chats::ChatsHeaderStatus;

fn url() -> Url {
    Url::from_string("wss://riseonly.net").expect("a literal test url parses")
}

#[test]
fn socket_status_has_priority_over_chat_refresh() {
    assert_eq!(
        ChatsHeaderStatus::resolve(&SaiWebSocketManagerPhase::Connecting(url()), true, false, false),
        ChatsHeaderStatus::Connecting
    );
    assert_eq!(
        ChatsHeaderStatus::resolve(&SaiWebSocketManagerPhase::Disconnected, true, false, false),
        ChatsHeaderStatus::WaitingForNetwork
    );
}

#[test]
fn connected_socket_shows_refresh_only_while_chats_are_in_flight() {
    let connected = SaiWebSocketManagerPhase::Connected(url(), Date::now());

    assert_eq!(ChatsHeaderStatus::resolve(&connected, true, false, false), ChatsHeaderStatus::Updating);
    assert_eq!(ChatsHeaderStatus::resolve(&connected, false, false, false), ChatsHeaderStatus::Title);
}
