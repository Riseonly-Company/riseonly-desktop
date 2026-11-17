use gpui::{App, FontWeight, IntoElement, ParentElement, RenderOnce, SharedString, Window, div};
use rise_ui::swiftui::font::{Font, FontDesign, SwiftUiFont};

use crate::core::ui::reaction_emoji_glyph::ReactionEmojiGlyph;
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatTopic;

/// topic icon, a unicode emoji or a custom emoji ref (ce:sticker_item_id)
/// unresolvable custom refs fall back to the default
#[derive(IntoElement)]
pub struct TopicIconGlyph {
    pub icon: SharedString,
    pub size: f32,
}

impl TopicIconGlyph {
    pub const DEFAULT_SIZE: f32 = 22.0;

    pub fn new(icon: impl Into<SharedString>) -> Self {
        Self {
            icon: icon.into(),
            size: Self::DEFAULT_SIZE,
        }
    }

    pub fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }
}

impl RenderOnce for TopicIconGlyph {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let size = self.size;
        ReactionEmojiGlyph::new(self.icon, size).fallback(move |_, _| {
            div()
                .swiftui_font(Font::system(size, FontWeight::NORMAL, FontDesign::Default))
                .child(ChatTopic::DEFAULT_ICON)
        })
    }
}
