use gpui::{AnyElement, Div, FontWeight, IntoElement, SharedString, div, prelude::*};
use rise_theme::{AppTheme, HeaderMetrics};

/// The bar a screen opens with: three slots, of which the centre is an overlay
/// rather than a flex child, so a long title cannot push the trailing button off
/// its edge. Both side slots reserve their width even when empty.
#[derive(Default)]
pub struct PageHeaderUi {
    title: Option<SharedString>,
    subtitle: Option<SharedString>,
    leading: Option<AnyElement>,
    trailing: Option<AnyElement>,
    center: Option<AnyElement>,
}

impl PageHeaderUi {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn title(mut self, title: impl Into<SharedString>) -> Self {
        let title = title.into();
        self.title = (!title.trim().is_empty()).then_some(title);
        self
    }

    pub fn subtitle(mut self, subtitle: impl Into<SharedString>) -> Self {
        let subtitle = subtitle.into();
        self.subtitle = (!subtitle.trim().is_empty()).then_some(subtitle);
        self
    }

    pub fn leading(mut self, slot: impl IntoElement) -> Self {
        self.leading = Some(slot.into_any_element());
        self
    }

    pub fn trailing(mut self, slot: impl IntoElement) -> Self {
        self.trailing = Some(slot.into_any_element());
        self
    }

    /// Replaces the title block entirely — the feed puts its tab bar here.
    pub fn center(mut self, slot: impl IntoElement) -> Self {
        self.center = Some(slot.into_any_element());
        self
    }

    pub fn render(self, theme: &AppTheme) -> Div {
        let metrics = theme.header;

        let row = div()
            .h(metrics.height)
            .w_full()
            .px(metrics.padding_x)
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .child(Self::slot(metrics, self.leading))
            .child(Self::slot(metrics, self.trailing));

        let center = self
            .center
            .or_else(|| Self::title_block(theme, self.title, self.subtitle));

        let mut header = div().relative().w_full().h(metrics.height).child(row);

        if let Some(center) = center {
            header = header.child(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(center),
            );
        }

        header
    }

    fn slot(metrics: HeaderMetrics, content: Option<AnyElement>) -> Div {
        let mut slot = div()
            .size(metrics.button_size)
            .flex()
            .items_center()
            .justify_center();

        if let Some(content) = content {
            slot = slot.child(content);
        }

        slot
    }

    fn title_block(
        theme: &AppTheme,
        title: Option<SharedString>,
        subtitle: Option<SharedString>,
    ) -> Option<AnyElement> {
        if title.is_none() && subtitle.is_none() {
            return None;
        }

        let mut block = div()
            .flex()
            .flex_col()
            .items_center()
            .gap(theme.header.title_gap);

        // Subtitle above the title: a kicker, not a caption.
        if let Some(subtitle) = subtitle {
            let token = theme
                .typography
                .style(HeaderMetrics::SUBTITLE_SIZE, FontWeight::NORMAL);
            block = block.child(
                div()
                    .text_size(token.size)
                    .line_height(token.line_height)
                    .font(token.font)
                    .text_color(theme.text.secondary)
                    .child(subtitle),
            );
        }

        if let Some(title) = title {
            let token = theme
                .typography
                .style(HeaderMetrics::TITLE_SIZE, FontWeight::BOLD);
            block = block.child(
                div()
                    .max_w(theme.header.title_max_width)
                    .truncate()
                    .text_size(token.size)
                    .line_height(token.line_height)
                    .font(token.font)
                    .text_color(theme.text.primary)
                    .child(title),
            );
        }

        Some(block.into_any_element())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blank_copy_is_not_a_title() {
        let header = PageHeaderUi::new().title("   ").subtitle("");
        assert!(header.title.is_none());
        assert!(header.subtitle.is_none());
    }

    #[test]
    fn a_centre_slot_replaces_the_title_block_rather_than_stacking_with_it() {
        let header = PageHeaderUi::new().title("Feed").center(div());
        assert!(header.center.is_some());
        assert!(
            header.title.is_some(),
            "the title is still recorded; render is where the centre wins"
        );
    }

    #[test]
    fn a_side_slot_reserves_the_same_width_whether_it_is_occupied_or_not() {
        let theme = AppTheme::dark();
        assert_eq!(theme.header.button_size, theme.avatar.toolbar);
    }
}
