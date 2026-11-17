use gpui::{AnyElement, App, Hsla, IntoElement, ParentElement, RenderOnce, Styled, Window, div};
use rise_theme::ThemeStore;
use smallvec::SmallVec;

use crate::swiftui::frame::SwiftUiAlignedFrame;
use crate::swiftui::stacks::{Alignment, zstack};

/// the page background with the content laid over it
#[derive(IntoElement)]
pub struct BgWrapperUi {
    use_bg: bool,
    bg_color: Option<Hsla>,
    extend_content_through_top: bool,
    extend_content_through_bottom: bool,
    content: SmallVec<[AnyElement; 1]>,
}

impl BgWrapperUi {
    pub fn new() -> Self {
        Self {
            use_bg: false,
            bg_color: None,
            extend_content_through_top: false,
            extend_content_through_bottom: false,
            content: SmallVec::new(),
        }
    }

    pub fn use_bg(mut self, use_bg: bool) -> Self {
        self.use_bg = use_bg;
        self
    }

    pub fn bg_color(mut self, bg_color: impl Into<Option<Hsla>>) -> Self {
        self.bg_color = bg_color.into();
        self
    }

    pub fn extend_content_through_top(mut self, extend: bool) -> Self {
        self.extend_content_through_top = extend;
        self
    }

    pub fn extend_content_through_bottom(mut self, extend: bool) -> Self {
        self.extend_content_through_bottom = extend;
        self
    }

    pub fn extends_content_through_top(&self) -> bool {
        self.extend_content_through_top
    }

    pub fn extends_content_through_bottom(&self) -> bool {
        self.extend_content_through_bottom
    }
}

impl Default for BgWrapperUi {
    fn default() -> Self {
        Self::new()
    }
}

impl ParentElement for BgWrapperUi {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.content.extend(elements);
    }
}

impl RenderOnce for BgWrapperUi {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let (bg_100, bg_200) = {
            let store = ThemeStore::shared(cx);
            let theme = store.read(cx).current_theme();
            (theme.bg._100, theme.bg._200)
        };
        let default_bg = if self.use_bg { bg_200 } else { bg_100 };
        zstack(Alignment::Center)
            .size_full()
            .grid_rows(1)
            .grid_cols(1)
            .child(div().size_full().bg(self.bg_color.unwrap_or(default_bg)))
            .child(
                div()
                    .size_full()
                    .children(self.content)
                    .frame_max(Alignment::Center),
            )
            .frame_max(Alignment::Center)
    }
}
