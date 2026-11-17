use gpui::{App, IntoElement, ParentElement, RenderOnce, Styled, Window, div, px, white};
use rise_theme::avatar_palette::AvatarPalette;
use rise_ui::IconUi;

#[derive(IntoElement)]
pub struct ChatFavouritesLogo {
    size: f32,
    icon_size: Option<f32>,
}

impl ChatFavouritesLogo {
    pub fn new(size: f32) -> Self {
        Self { size, icon_size: None }
    }

    pub fn icon_size(mut self, icon_size: impl Into<Option<f32>>) -> Self {
        self.icon_size = icon_size.into();
        self
    }
}

impl RenderOnce for ChatFavouritesLogo {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let icon_size = self.icon_size.unwrap_or_else(|| (self.size * 0.42).max(11.0));
        div()
            .size(px(self.size))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded_full()
            .bg(AvatarPalette::favourites_background())
            .children(IconUi::sized("bookmark.fill", px(icon_size), white(), true))
    }
}
