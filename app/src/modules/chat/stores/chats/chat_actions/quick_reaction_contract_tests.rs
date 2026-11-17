use rise_canon::rise_key::user as key;
use rise_canon::rise_request::user::UpdateProfile;
use serde_json::{Map, Value};

use crate::core::ui::emoji_picker_ui::emoji_picker_catalog::EmojiPickerCatalog;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatReactionsType, ChatSettingsModel};
use crate::modules::user::stores::user::user_actions::user_types::ProfileModel;
use crate::modules::user::stores::user::user_actions::user_wire_types::{
    RiseUserProfileRow, UserProfileSubscriptionDTO,
};

#[test]
fn update_profile_encodes_quick_reaction_wire_key() {
    let body = UpdateProfile { quick_reaction: Some("\u{2764}\u{FE0F}".to_owned()), ..UpdateProfile::default() };
    let json = match serde_json::to_value(&body) {
        Ok(Value::Object(object)) => object,
        other => panic!("update_profile body is not an object: {other:?}"),
    };

    assert!(json.get(key::USER_ID).is_none());
    assert_eq!(json.get(key::QUICK_REACTION).and_then(Value::as_str), Some("\u{2764}\u{FE0F}"));
}

#[test]
fn profile_decodes_persisted_quick_reaction() {
    let mut object = profile_object();
    object.insert(key::QUICK_REACTION.to_owned(), Value::from("\u{1F525}"));
    let dto = decode_profile(object);

    assert_eq!(dto.as_profile_model().quick_reaction, "\u{1F525}");
}

#[test]
fn missing_quick_reaction_uses_like_default() {
    let dto = decode_profile(profile_object());

    assert_eq!(dto.as_profile_model().quick_reaction, ProfileModel::DEFAULT_QUICK_REACTION);
}

#[test]
fn quick_reaction_requires_one_emoji_grapheme() {
    let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}\u{200D}\u{1F466}";
    let flag = "\u{1F1F0}\u{1F1FF}";
    assert_eq!(EmojiPickerCatalog::normalized_selectable_emoji(Some(family)).as_deref(), Some(family));
    assert_eq!(EmojiPickerCatalog::normalized_selectable_emoji(Some(flag)).as_deref(), Some(flag));
    assert_eq!(EmojiPickerCatalog::normalized_selectable_emoji(Some("\u{00A9}")).as_deref(), Some("\u{00A9}"));
    assert_eq!(EmojiPickerCatalog::normalized_selectable_emoji(Some("a")), None);
    assert_eq!(EmojiPickerCatalog::normalized_selectable_emoji(Some("1")), None);
    assert_eq!(EmojiPickerCatalog::normalized_selectable_emoji(Some("\u{1F44D}\u{1F44D}")), None);
}

#[test]
fn custom_emoji_ref_survives_the_read() {
    let reference = "ce:sticker_0123456789abcdef0123456789abcdef";
    assert_eq!(EmojiPickerCatalog::normalized_selectable_emoji(Some(reference)), None);
    assert_eq!(
        EmojiPickerCatalog::normalized_selectable_emoji_or_custom_ref(Some(reference)).as_deref(),
        Some(reference)
    );
    assert_eq!(
        EmojiPickerCatalog::normalized_selectable_emoji_or_custom_ref(Some("\u{1F44D}")).as_deref(),
        Some("\u{1F44D}")
    );
}

#[test]
fn custom_emoji_ref_matches_the_server_alphabet() {
    assert_eq!(
        EmojiPickerCatalog::custom_emoji_ref("sticker_0123456789abcdef0123456789abcdef").as_deref(),
        Some("ce:sticker_0123456789abcdef0123456789abcdef")
    );
    assert_eq!(
        EmojiPickerCatalog::custom_emoji_ref("579f1ba1-7d70-4e1f-8533-914523b08a5c").as_deref(),
        Some("ce:579f1ba1-7d70-4e1f-8533-914523b08a5c")
    );
    assert_eq!(EmojiPickerCatalog::custom_emoji_ref(""), None);
    assert_eq!(EmojiPickerCatalog::custom_emoji_ref("has space"), None);
    assert_eq!(EmojiPickerCatalog::custom_emoji_ref("../etc/passwd"), None);
    assert_eq!(EmojiPickerCatalog::custom_emoji_ref(&"a".repeat(62)), None);
}

#[test]
fn chat_reaction_config_allows_custom_emoji_only_when_the_list_does() {
    let mut model = ChatSettingsModel { chat_id: "chat-1".to_owned(), ..ChatSettingsModel::default() };
    model.reactions_type = ChatReactionsType::Custom;
    model.allowed_reactions = vec!["\u{1F44D}".to_owned(), "\u{1F525}".to_owned()];
    assert!(!model.reaction_config().allows_custom_emoji);

    model.allowed_reactions =
        vec!["\u{1F44D}".to_owned(), "ce:sticker_0123456789abcdef0123456789abcdef".to_owned()];
    assert!(model.reaction_config().allows_custom_emoji);

    model.reactions_type = ChatReactionsType::All;
    assert!(model.reaction_config().allows_custom_emoji);
}

fn profile_object() -> Map<String, Value> {
    let mut object = Map::new();
    object.insert(key::ID.to_owned(), Value::from("user-1"));
    object.insert(key::TAG.to_owned(), Value::from("alice"));
    object
}

fn decode_profile(object: Map<String, Value>) -> UserProfileSubscriptionDTO {
    match serde_json::from_value(Value::Object(object)) {
        Ok(dto) => dto,
        Err(error) => panic!("profile did not decode: {error}"),
    }
}
