use std::sync::Arc;

use gpui::{App, BoxShadow, FontWeight, Hsla, Window, point, white};
use rise_i18n::tr;
use rise_theme::avatar_palette::AvatarPalette;
use rise_theme::theme::theme_store::ThemeStore;
use rise_ui::liquid_chrome::LiquidChromeModifiers;
use rise_ui::swiftui::prelude::*;
use rise_ui::{IconUi, MainText, TextAlignT};

use crate::modules::chat::stores::chats::chat_actions::chat_avatar_palette::AvatarPaletteChat;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatInfo, ChatType};

struct Copy {
    icon: &'static str,
    icon_color: Hsla,
    icon_background: Hsla,
    title: String,
    subtitle: String,
}

#[derive(IntoElement)]
pub struct ChatMessagesEmptyState {
    chat: Arc<ChatInfo>,
    compact: bool,
}

impl ChatMessagesEmptyState {
    pub fn new(chat: Arc<ChatInfo>) -> Self {
        Self { chat, compact: false }
    }

    pub fn compact(mut self, compact: bool) -> Self {
        self.compact = compact;
        self
    }

    fn copy(&self) -> Copy {
        match self.chat.r#type {
            ChatType::Favourites => Copy {
                icon: "bookmark.fill",
                icon_color: white(),
                icon_background: AvatarPalette::favourites_background(),
                title: tr("chat_empty_favourites_title").to_string(),
                subtitle: tr("chat_empty_favourites_subtitle").to_string(),
            },
            ChatType::Group => Copy {
                icon: "person.2.fill",
                icon_color: white(),
                icon_background: <AvatarPalette as AvatarPaletteChat>::background_color(&self.chat),
                title: tr("chat_empty_group_title").to_string(),
                subtitle: tr("chat_empty_group_subtitle").to_string(),
            },
            ChatType::Channel => Copy {
                icon: "megaphone.fill",
                icon_color: white(),
                icon_background: <AvatarPalette as AvatarPaletteChat>::background_color(&self.chat),
                title: tr("chat_empty_channel_title").to_string(),
                subtitle: tr("chat_empty_channel_subtitle").to_string(),
            },
            ChatType::Direct => Copy {
                icon: "bubble.left.and.bubble.right.fill",
                icon_color: white(),
                icon_background: <AvatarPalette as AvatarPaletteChat>::background_color(&self.chat),
                title: tr("chat_empty_private_title").to_string(),
                subtitle: tr("chat_empty_private_subtitle").to_string(),
            },
        }
    }
}

impl RenderOnce for ChatMessagesEmptyState {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = ThemeStore::shared(cx).read(cx).current_theme().clone();
        let content = self.copy();

        let card = div()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(14.0))
            .px(px(24.0))
            .py(px(if self.compact { 18.0 } else { 28.0 }))
            .when(!self.compact, |card| {
                card.child(
                    div()
                        .size(px(72.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .bg(content.icon_background.opacity(0.92))
                        .shadow(vec![BoxShadow {
                            color: content.icon_background.opacity(0.35),
                            offset: point(px(0.0), px(8.0)),
                            blur_radius: px(18.0),
                            spread_radius: px(0.0),
                            inset: false,
                        }])
                        .children(IconUi::sized(content.icon, px(30.0), content.icon_color, true)),
                )
            })
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        MainText::new(content.title)
                            .px(18.0)
                            .tac(TextAlignT::Center)
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(theme.text.primary.opacity(0.92)),
                    )
                    .child(
                        div().w_full().max_w(px(280.0)).child(
                            MainText::new(content.subtitle)
                                .px(14.0)
                                .tac(TextAlignT::Center)
                                .color(theme.text.secondary.opacity(0.88))
                                .number_of_lines(3),
                        ),
                    ),
            )
            .liquid_chrome_rounded_rectangle(24.0);

        div().px(px(28.0)).child(card)
    }
}
