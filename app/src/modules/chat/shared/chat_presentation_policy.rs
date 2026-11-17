use std::collections::HashSet;

use rise_canon::RiseLimits;

use crate::app::router::navigation_types::RootRoute;
use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatType;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChatProfilePresentationMode {
    Full,
    ContentTabsOnly,
}

pub enum ChatPresentationPolicy {}

impl ChatPresentationPolicy {
    pub fn allows_list_context_menu(_type: ChatType) -> bool {
        true
    }

    pub fn profile_mode(r#type: ChatType) -> ChatProfilePresentationMode {
        if r#type == ChatType::Favourites {
            ChatProfilePresentationMode::ContentTabsOnly
        } else {
            ChatProfilePresentationMode::Full
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChatProfileOpenChatDecision {
    Unwind { depth: i64 },
    Push,
}

pub enum ChatProfileNavigationPolicy {}

impl ChatProfileNavigationPolicy {
    /// open chat acts as back only when the conversation is behind the profile
    /// from member list or search or deep link nothing is behind it, so it pushes
    /// chat_ids has every id the chat answered to
    pub fn open_chat_decision(
        path: &[RootRoute],
        chat_ids: &HashSet<String>,
        embedded_forum_chat_id: Option<&str>,
    ) -> ChatProfileOpenChatDecision {
        let identifiers = Self::non_empty_identifiers(chat_ids);
        if identifiers.is_empty() {
            return ChatProfileOpenChatDecision::Push;
        }

        if path.len() > 1 {
            for index in (0..=path.len() - 2).rev() {
                if Self::presents_conversation(&path[index], &identifiers) {
                    return ChatProfileOpenChatDecision::Unwind { depth: index as i64 + 1 };
                }
            }
        }
        if embedded_forum_chat_id.is_some_and(|id| identifiers.contains(id)) {
            return ChatProfileOpenChatDecision::Unwind { depth: 0 };
        }
        ChatProfileOpenChatDecision::Push
    }

    pub fn conversation_is_directly_behind(
        path: &[RootRoute],
        chat_ids: &HashSet<String>,
        embedded_forum_chat_id: Option<&str>,
    ) -> bool {
        let identifiers = Self::non_empty_identifiers(chat_ids);
        if identifiers.is_empty() || path.is_empty() {
            return false;
        }
        if path.len() == 1 {
            return embedded_forum_chat_id.is_some_and(|id| identifiers.contains(id));
        }
        Self::presents_conversation(&path[path.len() - 2], &identifiers)
    }

    fn non_empty_identifiers(chat_ids: &HashSet<String>) -> HashSet<&str> {
        chat_ids.iter().map(String::as_str).filter(|id| !id.is_empty()).collect()
    }

    fn presents_conversation(route: &RootRoute, chat_ids: &HashSet<&str>) -> bool {
        match route {
            RootRoute::Chat { chat_id }
            | RootRoute::ChatTopic { chat_id, .. }
            | RootRoute::ChatThread { chat_id, .. }
            | RootRoute::ForumHome { chat_id }
            | RootRoute::ForumAll { chat_id } => {
                chat_id.as_deref().is_some_and(|id| !id.is_empty() && chat_ids.contains(id))
            }
            _ => false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ChatProfileAction {
    OpenChat,
    Profile,
    GroupVoice,
    Call,
    Notifications,
    Gift,
    Search,
    More,
}

/// layout of the chat profile action row
/// search and more are always the two rightmost and profile is never dropped
/// what doesnt fit leaves from the button nearest search and goes to the top of more
pub enum ChatProfileActionRowPolicy {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatProfileActionRowPolicyLayout {
    pub visible: Vec<ChatProfileAction>,
    pub overflow: Vec<ChatProfileAction>,
}

impl ChatProfileActionRowPolicy {
    pub fn max_visible_buttons() -> i64 {
        RiseLimits::client_value("chat_profile_action_row_max_buttons")
    }

    /// max_visible none means the default max visible buttons
    pub fn layout(candidates: &[ChatProfileAction], max_visible: Option<i64>) -> ChatProfileActionRowPolicyLayout {
        const TAIL: [ChatProfileAction; 2] = [ChatProfileAction::Search, ChatProfileAction::More];
        let max_visible = max_visible.unwrap_or_else(Self::max_visible_buttons);
        let mut head: Vec<ChatProfileAction> =
            candidates.iter().copied().filter(|action| !TAIL.contains(action)).collect();
        let capacity = (max_visible - TAIL.len() as i64).max(0) as usize;
        let mut overflow = Vec::new();
        while head.len() > capacity {
            let Some(index) = head.iter().rposition(|action| *action != ChatProfileAction::Profile) else {
                break;
            };
            overflow.insert(0, head.remove(index));
        }
        head.extend(TAIL);
        ChatProfileActionRowPolicyLayout { visible: head, overflow }
    }
}
