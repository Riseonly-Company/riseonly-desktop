use rise_canon::rise_key::chat as key;
use rise_core::foundation::foundation_url::Url;
use serde_json::{Map, Value};

use super::chat_wire_types::{GetChatPreviewGatewayDTO, GetChatPreviewGatewayDTOExt};
use crate::core::config::app_environment::AppEnvironment;
use crate::core::lib::sai::core::utilities::sai_payload_path::SaiJSONCodec;
use crate::core::push_notifications::push_deep_link_store::PushDeepLink;
use crate::core::push_notifications::universal_link_parser::UniversalLinkParser;

fn url(raw: &str) -> Url {
    match Url::from_string(raw) {
        Some(url) => url,
        None => panic!("not a URL: {raw}"),
    }
}

fn join_chat(token: &str) -> Option<PushDeepLink> {
    Some(PushDeepLink::JoinChat { token: token.to_owned() })
}

#[test]
fn production_join_link_parses_invite_token() {
    let token = "4fdb52a567cb4c989804068d2ec77b66";
    let url = url(&format!("https://riseonly.net/join/{token}"));

    assert_eq!(UniversalLinkParser::parse(&url, AppEnvironment::Prod), join_chat(token));
}

#[test]
fn supported_join_link_forms_use_the_same_route() {
    let token = "Abcd1234_token.value~";
    let web = url(&format!("https://riseonly.net/join/{token}"));
    let custom = url(&format!("riseonly://join/{token}"));
    let query = url(&format!("riseonly://join?token={token}"));

    assert_eq!(UniversalLinkParser::parse(&web, AppEnvironment::Prod), join_chat(token));
    assert_eq!(UniversalLinkParser::parse(&custom, AppEnvironment::Prod), join_chat(token));
    assert_eq!(UniversalLinkParser::parse(&query, AppEnvironment::Prod), join_chat(token));
}

#[test]
fn malformed_join_links_are_rejected() {
    let token = "4fdb52a567cb4c989804068d2ec77b66";
    let urls: Vec<Url> = [
        format!("http://riseonly.net/join/{token}"),
        format!("https://www.riseonly.net/join/{token}"),
        format!("https://example.com/join/{token}"),
        format!("https://riseonly.net/join/{token}/extra"),
        "https://riseonly.net/join/short".to_owned(),
        "https://riseonly.net/join/invalid+token-value".to_owned(),
        "https://riseonly.net/join/invalid%2Ftoken".to_owned(),
        format!("https://riseonly.net/join/%20{token}"),
        format!("https://riseonly.net/join/{token}%0A"),
        format!("riseonly://join?token=%20{token}"),
    ]
    .iter()
    .map(|raw| url(raw))
    .collect();

    for url in urls {
        assert_eq!(UniversalLinkParser::parse(&url, AppEnvironment::Prod), None);
    }
}

#[test]
fn invite_preview_maps_approval_and_pending_states() {
    crate::install_feature_hooks();
    let approval = SaiJSONCodec::decode::<GetChatPreviewGatewayDTO>(Some(&preview_payload(
        "requires_approval",
        true,
        true,
    )))
    .and_then(|preview| preview.to_chat_join_preview());
    let pending = SaiJSONCodec::decode::<GetChatPreviewGatewayDTO>(Some(&preview_payload(
        "join_request_pending",
        false,
        true,
    )))
    .and_then(|preview| preview.to_chat_join_preview());
    let Some(approval) = approval else { panic!("approval preview did not decode") };
    let Some(pending) = pending else { panic!("pending preview did not decode") };

    assert!(approval.can_join);
    assert!(approval.requires_approval);
    assert_eq!(approval.join_status, "requires_approval");
    assert!(!pending.can_join);
    assert!(!pending.requires_approval);
    assert_eq!(pending.join_status, "join_request_pending");
}

fn preview_payload(status: &str, can_join: bool, creates_join_request: bool) -> Value {
    let mut chat = Map::new();
    chat.insert(key::ID.to_owned(), Value::from("chat-1"));
    chat.insert(key::TITLE.to_owned(), Value::from("Riseonly Chat"));
    chat.insert(key::TYPE.to_owned(), Value::from("GROUP"));
    chat.insert(key::MEMBER_COUNT.to_owned(), Value::from(42));
    let mut invite_link_info = Map::new();
    invite_link_info.insert(key::ID.to_owned(), Value::from("link-1"));
    invite_link_info.insert(key::CREATES_JOIN_REQUEST.to_owned(), Value::from(creates_join_request));
    let mut payload = Map::new();
    payload.insert(key::SUCCESS.to_owned(), Value::from(true));
    payload.insert(key::CHAT.to_owned(), Value::Object(chat));
    payload.insert(key::IS_MEMBER.to_owned(), Value::from(false));
    payload.insert(key::CAN_JOIN.to_owned(), Value::from(can_join));
    payload.insert(key::JOIN_STATUS.to_owned(), Value::from(status));
    payload.insert(key::INVITE_LINK_INFO.to_owned(), Value::Object(invite_link_info));
    Value::Object(payload)
}
