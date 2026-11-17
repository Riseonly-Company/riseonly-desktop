use std::collections::HashSet;

use rise_chat::ChatType;

use crate::modules::chat::components::chat_item::chat_item::ChatListMenuPolicy;
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_tab::ChatProfileTab;
use crate::modules::chat::shared::chat_presentation_policy::{ChatPresentationPolicy, ChatProfilePresentationMode};
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatInfo;

#[test]
fn favourites_chat_has_restricted_list_context_menu_and_uses_content_tabs_only_profile() {
    assert!(ChatPresentationPolicy::allows_list_context_menu(ChatType::Favourites));
    let mut chat = ChatInfo::new("favourites".to_owned());
    chat.r#type = ChatType::Favourites;
    let actions = ChatListMenuPolicy::actions(&chat, 0, false, false, false, Vec::new(), &HashSet::new());
    let ids: Vec<&str> = actions.iter().map(|action| action.id.as_str()).collect();
    assert_eq!(ids, ["pin", "delete"]);
    assert_eq!(ChatPresentationPolicy::profile_mode(ChatType::Favourites), ChatProfilePresentationMode::ContentTabsOnly);
    let ordered = ChatProfileTab::ordered(ChatType::Favourites, false, false, true);
    assert_eq!(
        ordered,
        [
            ChatProfileTab::Media,
            ChatProfileTab::Fav,
            ChatProfileTab::File,
            ChatProfileTab::Link,
            ChatProfileTab::Music,
            ChatProfileTab::Voice,
            ChatProfileTab::Gif,
        ]
    );
    assert!(!ordered.contains(&ChatProfileTab::Groups));
    assert!(!ordered.contains(&ChatProfileTab::Members));
}

#[test]
fn ordinary_chats_keep_their_full_presentation() {
    for chat_type in [ChatType::Direct, ChatType::Group, ChatType::Channel] {
        assert!(ChatPresentationPolicy::allows_list_context_menu(chat_type));
        assert_eq!(ChatPresentationPolicy::profile_mode(chat_type), ChatProfilePresentationMode::Full);
    }
}
