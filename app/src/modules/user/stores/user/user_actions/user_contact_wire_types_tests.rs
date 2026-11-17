use rise_canon::rise_key::user as user_key;
use rise_canon::{rise_request, rise_wire_methods};
use serde_json::Value;

use crate::core::lib::sai::core::websocket::sai_ws_deferred_outbound_coordinator::SaiWsDeferredDiskProtection;
use crate::modules::chat::pages::chats::chat::chat_profile::chat_contact_profile_policy::ChatContactProfilePolicy;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatInfo, ChatType, User};
use crate::modules::user::shared::contact_device_book::ContactDeviceStateStore;
use crate::modules::user::stores::user::user_actions::user_contact_types::{UserContactDraft, UserContactLimits};
use crate::modules::user::stores::user::user_actions::user_contact_wire_types::{
    SavedContactDtoExt, UserContactUserWireDTO, UserSavedContactWireDTO,
};
use crate::modules::user::stores::user::user_services::user_services::UserServicesStore;

#[test]
fn test_contact_profile_eligibility_matrix() {
    let current_user_id = "me";
    let mut private_peer = ChatInfo::new("chat".to_owned());
    private_peer.r#type = ChatType::Direct;
    private_peer.participant = User { id: "peer".to_owned(), name: "Peer".to_owned(), ..User::default() };
    assert!(ChatContactProfilePolicy::is_eligible(&private_peer, current_user_id, false));

    private_peer.participant.is_bot = true;
    assert!(!ChatContactProfilePolicy::is_eligible(&private_peer, current_user_id, false));

    private_peer.participant.is_bot = false;
    private_peer.participant.id = current_user_id.to_owned();
    assert!(!ChatContactProfilePolicy::is_eligible(&private_peer, current_user_id, false));

    private_peer.participant.id = "peer".to_owned();
    private_peer.r#type = ChatType::Group;
    assert!(!ChatContactProfilePolicy::is_eligible(&private_peer, current_user_id, false));

    private_peer.r#type = ChatType::Direct;
    assert!(!ChatContactProfilePolicy::is_eligible(&private_peer, current_user_id, true));
}

#[test]
fn test_recipient_only_identifiers_are_namespaced() {
    assert_eq!(UserServicesStore::contact_join_virtual_chat_id("42"), "contact-join:42");
    assert_eq!(UserServicesStore::contact_join_local_message_id("42"), "contact-join-event:42");
    assert!(UserServicesStore::is_contact_join_virtual_chat_id("contact-join:42"));
    assert!(!UserServicesStore::is_contact_join_virtual_chat_id("42"));
}

#[test]
fn test_contact_draft_normalizes_limits_and_requires_first_name() {
    let long_name = "a".repeat(usize::try_from(UserContactLimits::name() + 10).unwrap_or_default());
    let draft = UserContactDraft {
        target_user_id: "peer".to_owned(),
        first_name: format!("  {long_name}  "),
        last_name: " Family ".to_owned(),
        note: " Note ".to_owned(),
        share_phone_number: false,
        sync_to_phone: true,
    };
    assert!(draft.is_valid());
    assert_eq!(i64::try_from(draft.normalized_first_name().chars().count()).ok(), Some(UserContactLimits::name()));
    assert_eq!(draft.normalized_last_name(), "Family");
    assert_eq!(draft.normalized_note(), "Note");

    let mut invalid = draft.clone();
    invalid.first_name = "   ".to_owned();
    assert!(!invalid.is_valid());
}

#[test]
fn test_saved_contact_uses_alias_and_avatar_override() {
    let user = UserContactUserWireDTO {
        id: "peer".to_owned(),
        is_bot: false,
        is_deleted: false,
        is_online: false,
        is_premium: false,
        last_seen: None,
        logo: "https://example.test/public.jpg".to_owned(),
        name: "Public Name".to_owned(),
        tag: "peer".to_owned(),
        user_chat_id: String::new(),
    };
    let contact = UserSavedContactWireDTO {
        avatar_override_asset_id: Some("asset".to_owned()),
        avatar_override_url: Some("https://example.test/local.jpg".to_owned()),
        created_at_ms: 1,
        display_name: String::new(),
        first_name: "Local".to_owned(),
        id: "saved".to_owned(),
        last_name: "Alias".to_owned(),
        note: String::new(),
        share_phone_number: false,
        source: "MANUAL".to_owned(),
        sync_to_phone: true,
        target_user_id: "peer".to_owned(),
        updated_at_ms: 2,
        user: Some(user),
    };
    assert_eq!(contact.resolved_display_name(), "Local Alias");
    assert_eq!(contact.effective_avatar_url(), "https://example.test/local.jpg");
}

#[test]
fn test_generation_never_regresses_when_snapshot_is_lost_or_stale() {
    assert_eq!(ContactDeviceStateStore::next_generation_value(8, 0), Some(9));
    assert_eq!(ContactDeviceStateStore::next_generation_value(8, 12), Some(13));
    assert_eq!(ContactDeviceStateStore::next_generation_value(i64::MAX, 0), None);
}

#[test]
fn test_contact_join_read_ack_uses_only_exact_event_ids() -> serde_json::Result<()> {
    let body = rise_request::user::MarkContactJoinEventsRead {
        event_ids: Some(vec!["event-2".to_owned(), "event-1".to_owned()]),
    };
    let object = serde_json::to_value(&body)?;
    assert_eq!(object.get(user_key::EVENT_IDS), Some(&Value::from(vec!["event-2", "event-1"])));
    assert!(object.get("through_id").is_none());
    assert!(object.get(user_key::USER_ID).is_none());
    Ok(())
}

#[test]
fn test_sync_batch_ceiling_matches_server_snapshot_limit() {
    assert_eq!(UserContactLimits::fallback_sync_batch(), 500);
    assert_eq!(UserContactLimits::maximum_sync_batches(), 50);
    assert_eq!(UserContactLimits::MAXIMUM_SYNC_RETRY_ATTEMPTS, 8);
}

#[test]
fn test_saved_contact_folder_automatic_hydration_is_bounded() {
    assert_eq!(UserContactLimits::maximum_automatic_saved_contact_pages(), 3);
    assert_eq!(UserContactLimits::maximum_automatic_saved_contact_ids(), 300);
    assert_eq!(UserContactLimits::maximum_visible_saved_contact_hydrations(), 20);
    assert!(
        UserContactLimits::maximum_automatic_saved_contact_pages() * UserContactLimits::list_page()
            <= UserContactLimits::maximum_automatic_saved_contact_ids()
    );
}

#[test]
fn test_contact_requests_are_classified_as_memory_only() {
    for method in [
        rise_wire_methods::user::UPDATE_CONTACT_SYNC_SETTINGS,
        rise_wire_methods::user::SYNC_CONTACTS,
        rise_wire_methods::user::UPSERT_SAVED_CONTACT,
        rise_wire_methods::user::DELETE_SAVED_CONTACT,
        rise_wire_methods::user::SET_SAVED_CONTACT_PHOTO,
        rise_wire_methods::user::SUGGEST_CONTACT_PHOTO,
        rise_wire_methods::user::RESOLVE_CONTACT_PHOTO_SUGGESTION,
        rise_wire_methods::user::BLOCK_USER,
        rise_wire_methods::user::UNBLOCK_USER,
        rise_wire_methods::user::MARK_CONTACT_JOIN_EVENTS_READ,
    ] {
        assert!(SaiWsDeferredDiskProtection::must_remain_memory_only(rise_wire_methods::user::SERVICE, method));
    }
    assert!(!SaiWsDeferredDiskProtection::must_remain_memory_only(
        rise_wire_methods::message::SERVICE,
        rise_wire_methods::message::CREATE_MESSAGE,
    ));
}
