use crate::modules::chat::components::chat::bot_chat_menu_sheet::BotChatMenuPresentationContract;
use crate::modules::chat::components::chat::composer::chat_composer_types::ChatBottomBarMetrics;

#[test]
fn menu_button_matches_composer_control_size() {
    assert_eq!(BotChatMenuPresentationContract::BUTTON_SIZE, ChatBottomBarMetrics::CIRCLE);
    assert_eq!(BotChatMenuPresentationContract::BUTTON_SIZE, 42.0);
}

#[test]
fn software_keyboard_selects_expanded_detent() {
    assert!(!BotChatMenuPresentationContract::starts_expanded(0.0));
    assert!(BotChatMenuPresentationContract::starts_expanded(336.0));

    assert_eq!(
        BotChatMenuPresentationContract::initial_snap_fraction(false),
        BotChatMenuPresentationContract::COLLAPSED_FRACTION
    );
    assert_eq!(
        BotChatMenuPresentationContract::initial_snap_fraction(true),
        BotChatMenuPresentationContract::EXPANDED_FRACTION
    );
    assert!(BotChatMenuPresentationContract::EXPANDED_FRACTION > BotChatMenuPresentationContract::COLLAPSED_FRACTION);
    assert!(BotChatMenuPresentationContract::EXPANDED_FRACTION <= 1.0);
}

#[test]
fn menu_occludes_header_accessory() {
    assert_eq!(BotChatMenuPresentationContract::header_accessory_opacity(true), 0.0);
    assert_eq!(BotChatMenuPresentationContract::header_accessory_opacity(false), 1.0);
}
