use gpui::{AnyElement, Div, IntoElement, div, prelude::*};
use rise_theme::AppTheme;

use crate::page_header::PageHeaderUi;

/// A screen: a header, an optional strip under it, and content that fills the
/// rest. They are stacked, not overlaid, so the content's scroll never has to
/// know the header's height.
#[derive(Default)]
pub struct ScreenShellUi {
    header: Option<AnyElement>,
    below_header: Option<AnyElement>,
    content: Option<AnyElement>,
}

impl ScreenShellUi {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn header(mut self, header: PageHeaderUi, theme: &AppTheme) -> Self {
        self.header = Some(header.render(theme).into_any_element());
        self
    }

    /// Anything that belongs to the chrome rather than to the content — the
    /// stories strip, a filter row. It scrolls with neither.
    pub fn below_header(mut self, slot: impl IntoElement) -> Self {
        self.below_header = Some(slot.into_any_element());
        self
    }

    pub fn content(mut self, content: impl IntoElement) -> Self {
        self.content = Some(content.into_any_element());
        self
    }

    /// A screen paints NO background of its own.
    ///
    /// The shell's plate under it already is the app's colour, so a fill here
    /// only adds overdraw — and at the plate's edge it squares off the rounded
    /// corner, since gpui's content mask is a rectangle and cannot clip to a
    /// radius. `theme` stays in the signature because the header needs it.
    pub fn render(self, theme: &AppTheme) -> Div {
        let _ = theme;
        let mut screen = div().size_full().flex().flex_col().overflow_hidden();

        if let Some(header) = self.header {
            screen = screen.child(div().flex_shrink_0().child(header));
        }

        if let Some(strip) = self.below_header {
            screen = screen.child(div().flex_shrink_0().child(strip));
        }

        // A flex child's default minimum is its content, so without `min_h_0` a
        // long list pushes the header off the top instead of scrolling.
        screen.child(
            div()
                .flex_1()
                .min_h_0()
                .child(self.content.unwrap_or_else(|| div().into_any_element())),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shell_with_no_content_still_builds_a_screen() {
        let theme = AppTheme::dark();
        let _ = ScreenShellUi::new()
            .header(PageHeaderUi::new().title("Feed"), &theme)
            .render(&theme);
    }
}
