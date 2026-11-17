use gpui::Hsla;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_theme::avatar_palette::AvatarPalette;

use super::chat_types::{ChatInfo, ChatType};

/// chat overloads of the avatar palette
/// palette lives in rise-theme which cant see the chat model so they ride this trait
pub trait AvatarPaletteChat {
    fn background_color(chat: &ChatInfo) -> Hsla;

    fn display_letter_for_chat(chat: &ChatInfo) -> String;

    fn resolved_logo_url_for_chat(chat: &ChatInfo) -> Option<String>;
}

impl AvatarPaletteChat for AvatarPalette {
    fn background_color(chat: &ChatInfo) -> Hsla {
        match chat.r#type {
            ChatType::Favourites => AvatarPalette::favourites_background(),
            ChatType::Group | ChatType::Channel => AvatarPalette::color_for_chat(&chat.id),
            ChatType::Direct => {
                let user_id = trimming_whitespaces_and_newlines(&chat.participant.id);
                AvatarPalette::color_for_user(
                    if user_id.is_empty() { None } else { Some(user_id) },
                    Some(&chat.participant.name),
                )
            }
        }
    }

    fn display_letter_for_chat(chat: &ChatInfo) -> String {
        let source: &str = match chat.r#type {
            ChatType::Direct if chat.has_no_direct_peer() => &chat.title,
            ChatType::Direct => {
                if !chat.participant.name.is_empty() {
                    &chat.participant.name
                } else {
                    &chat.participant.tag
                }
            }
            _ => &chat.title,
        };
        AvatarPalette::display_letter(Some(source))
    }

    fn resolved_logo_url_for_chat(chat: &ChatInfo) -> Option<String> {
        match chat.r#type {
            ChatType::Favourites => None,
            ChatType::Direct => AvatarPalette::resolved_logo_url(Some(&chat.participant.more.logo)),
            ChatType::Group | ChatType::Channel => AvatarPalette::resolved_logo_url(chat.logo_url.as_deref()),
        }
    }
}
