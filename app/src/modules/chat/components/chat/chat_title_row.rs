use gpui::{App, FontWeight, SharedString, Window};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::tr;
use rise_ui::IconUi;
use rise_ui::bot_icon::BotIcon;
use rise_ui::main_text::MainText;
use rise_ui::official_badge::OfficialBadge;
use rise_ui::swiftui::prelude::*;

use crate::core::ui::premium_badge::PremiumBadge;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatInfo, ChatType};
use crate::modules::user::stores::user::user_actions::user_contact_wire_types::SavedContactDtoExt;
use crate::modules::user::stores::user::user_services::user_services::UserServicesStore;

const BOT_ICON_MINIMUM_SIZE: f32 = 10.0;
const BOT_ICON_SCALE: f32 = 0.72;

#[derive(IntoElement)]
pub struct ChatTitleRow {
    chat: ChatInfo,
    title_px: f32,
    fav_px: f32,
    icon_size: f32,
    font_weight: FontWeight,
}

impl ChatTitleRow {
    pub fn new(chat: ChatInfo) -> Self {
        Self { chat, title_px: 16.0, fav_px: 16.0, icon_size: 12.0, font_weight: FontWeight::SEMIBOLD }
    }

    pub fn title_px(mut self, title_px: f32) -> Self {
        self.title_px = title_px;
        self
    }

    pub fn fav_px(mut self, fav_px: f32) -> Self {
        self.fav_px = fav_px;
        self
    }

    pub fn icon_size(mut self, icon_size: f32) -> Self {
        self.icon_size = icon_size;
        self
    }

    pub fn font_weight(mut self, font_weight: FontWeight) -> Self {
        self.font_weight = font_weight;
        self
    }

    /// plain text name this row shows, for places that print the chat without the row
    pub fn plain_title(chat: &ChatInfo, cx: &mut App) -> String {
        match chat.r#type {
            ChatType::Favourites => tr("chats_favourites"),
            ChatType::Direct => {
                if chat.participant.is_deleted {
                    return tr("chat_deleted_account");
                }
                let alias = UserServicesStore::observed_saved_contact(&chat.participant.id, cx)
                    .map(|contact| trimming_whitespaces_and_newlines(&contact.resolved_display_name()).to_owned())
                    .unwrap_or_default();
                if !alias.is_empty() {
                    return alias;
                }
                chat.direct_peer_profile_name()
            }
            _ => chat.title.clone(),
        }
    }

    fn type_icon_symbol(chat_type: ChatType) -> Option<&'static str> {
        match chat_type {
            ChatType::Group => Some("person.2.fill"),
            ChatType::Channel => Some("megaphone.fill"),
            ChatType::Favourites => Some("bookmark.fill"),
            ChatType::Direct => None,
        }
    }
}

impl RenderOnce for ChatTitleRow {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();

        let chat = &self.chat;
        let title = MainText::new(Self::plain_title(chat, cx))
            .px(if chat.r#type == ChatType::Favourites { self.fav_px } else { self.title_px })
            .font_weight(self.font_weight)
            .number_of_lines(1)
            .min_w_0();

        let participant = (chat.r#type == ChatType::Direct).then_some(&chat.participant);
        let bot_icon = participant.filter(|participant| participant.is_bot).map(|_| {
            BotIcon::new(theme.text.primary).size((self.title_px * BOT_ICON_SCALE).max(BOT_ICON_MINIMUM_SIZE))
        });
        let official_badge = participant
            .filter(|participant| participant.more.is_official)
            .map(|_| OfficialBadge::new().size(px(self.title_px)));
        let premium_badge = participant.filter(|participant| participant.more.is_premium).map(|participant| {
            PremiumBadge::new()
                .user_id(Some(SharedString::from(participant.id.clone())))
                .emoji(participant.more.premium_badge_emoji.clone().map(SharedString::from))
                .size(self.title_px)
        });
        let type_icon = Self::type_icon_symbol(chat.r#type)
            .and_then(|symbol| IconUi::sized(symbol, px(self.icon_size), theme.text.secondary, true));

        hstack(4.0)
            .min_w_0()
            .child(title)
            .children(bot_icon)
            .children(official_badge)
            .children(premium_badge)
            .children(type_icon)
    }
}
