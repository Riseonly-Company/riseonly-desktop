use gpui::{App, ElementId, FontWeight, IntoElement, RenderOnce, SharedString, Window};
use rise_theme::theme::theme_environment::theme_store;
use rise_ui::main_text::MainText;
use rise_ui::swiftui::prelude::*;

use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::modules::chat::stores::message::message_actions::message_types::MessageContactInfo;

#[derive(IntoElement)]
pub struct MessageContactView {
    id: ElementId,
    pub contact: MessageContactInfo,
    pub is_own: bool,
}

impl MessageContactView {
    pub fn new(id: impl Into<ElementId>, contact: MessageContactInfo) -> Self {
        Self {
            id: id.into(),
            contact,
            is_own: false,
        }
    }

    pub fn is_own(mut self, is_own: bool) -> Self {
        self.is_own = is_own;
        self
    }
}

impl RenderOnce for MessageContactView {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let store = theme_store(cx);
        let theme = store.read(cx).current_theme();
        let name_color = if self.is_own { theme.content_on(theme.primary._100) } else { theme.text.primary };
        let phone_color = if self.is_own { theme.content_on(theme.primary._100).opacity(0.78) } else { theme.text.secondary };

        hstack(7.0)
            .child(
                UserLogoUi::new(self.id.clone())
                    .source(self.contact.avatar_url.clone().map(SharedString::from))
                    .name(SharedString::from(self.contact.name.clone()))
                    .size(40.0),
            )
            .child(
                vstack(2.0)
                    .align(HorizontalAlignment::Leading)
                    .child(
                        MainText::new(self.contact.name.clone())
                            .px(14.0)
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(name_color)
                            .number_of_lines(1),
                    )
                    .child(
                        MainText::new(self.contact.phone.clone())
                            .px(13.0)
                            .color(phone_color)
                            .number_of_lines(1),
                    ),
            )
            .child(spacer_min_w(0.0))
            .w_full()
            .max_w(px(280.0))
    }
}
