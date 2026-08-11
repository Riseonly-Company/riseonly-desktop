use gpui::{
    AnyElement, ClickEvent, Div, FontWeight, IntoElement, SharedString, Stateful, div, prelude::*,
};
use rise_i18n::tr;
use rise_theme::AppTheme;
use rise_ui::{IconSize, IconUi, MainText, TextTone};

/// What a screen backed by the network is currently able to show. The cache
/// decides which state is drawn, not the request: a refresh that fails while
/// rows are on screen is an error beside the rows, never instead of them.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum PageStatus {
    /// Nothing has been asked for yet.
    #[default]
    Idle,
    Loading,
    Loaded,
    Failed,
}

impl PageStatus {
    /// What to draw, given the status and whether anything is cached.
    pub fn presentation(self, has_items: bool) -> PagePresentation {
        if has_items {
            return match self {
                Self::Failed => PagePresentation::ContentWithRefreshError,
                _ => PagePresentation::Content,
            };
        }

        match self {
            Self::Idle | Self::Loading => PagePresentation::Loading,
            Self::Loaded => PagePresentation::Empty,
            Self::Failed => PagePresentation::Error,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PagePresentation {
    Loading,
    Empty,
    Error,
    Content,
    ContentWithRefreshError,
}

/// The three states every screen with backend data has to have.
pub struct PageStateUi;

impl PageStateUi {
    /// The full-height empty state: an icon and a line of copy.
    pub fn empty(theme: &AppTheme, sf_symbol: &str, text: impl Into<SharedString>) -> Div {
        let mut column = div()
            .w_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(theme.spacing._600)
            .px(theme.spacing._900)
            .py(theme.spacing._900);

        if let Some(icon) = IconUi::render(theme, sf_symbol, IconSize::Large, theme.text.secondary)
        {
            column = column.child(icon);
        }

        column.child(
            MainText::body(theme, TextTone::Secondary)
                .max_w(theme.feed.content_width)
                .text_center()
                .child(text.into()),
        )
    }

    /// The full-height error state, with a retry.
    pub fn error<V: 'static>(
        theme: &AppTheme,
        text: impl Into<SharedString>,
        retry: impl Fn(&mut V, &mut gpui::Window, &mut gpui::Context<V>) + 'static,
        cx: &mut gpui::Context<V>,
    ) -> Div {
        let mut column = div()
            .w_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(theme.spacing._600)
            .px(theme.spacing._900)
            .py(theme.spacing._900);

        if let Some(icon) = IconUi::render(
            theme,
            "wifi.exclamationmark",
            IconSize::Large,
            theme.text.secondary,
        ) {
            column = column.child(icon);
        }

        let token = theme
            .typography
            .style(rise_theme::FeedMetrics::CONTENT_SIZE, FontWeight::SEMIBOLD);

        column
            .child(
                MainText::body(theme, TextTone::Secondary)
                    .max_w(theme.feed.content_width)
                    .text_center()
                    .child(text.into()),
            )
            .child(
                div()
                    .id("page_state.retry")
                    .px(theme.spacing._800)
                    .py(theme.spacing._400)
                    .rounded(theme.button.radius_300)
                    .bg(theme.primary._100)
                    .text_size(token.size)
                    .line_height(token.line_height)
                    .font(token.font)
                    .text_color(theme.bg._000)
                    .cursor_pointer()
                    .on_click(
                        cx.listener(move |view, _: &ClickEvent, window, cx| {
                            retry(view, window, cx)
                        }),
                    )
                    .child(tr("common_retry")),
            )
    }

    /// The non-blocking failure: a pill laid over content that is still there,
    /// so its arrival moves nothing the user was reading.
    pub fn refresh_error<V: 'static>(
        theme: &AppTheme,
        text: impl Into<SharedString>,
        retry: impl Fn(&mut V, &mut gpui::Window, &mut gpui::Context<V>) + 'static,
        cx: &mut gpui::Context<V>,
    ) -> Stateful<Div> {
        let token = theme
            .typography
            .style(rise_theme::FeedMetrics::META_SIZE, FontWeight::MEDIUM);

        let mut pill = div()
            .id("page_state.refresh_error")
            .flex()
            .flex_row()
            .items_center()
            .gap(theme.spacing._300)
            .px(theme.spacing._600)
            .py(theme.spacing._400)
            .rounded(theme.button.radius_300)
            .bg(theme.bg._300)
            .border(theme.avatar.border_width)
            .border_color(theme.border._200)
            .cursor_pointer()
            .on_click(cx.listener(move |view, _: &ClickEvent, window, cx| retry(view, window, cx)));

        if let Some(icon) = IconUi::render(
            theme,
            "arrow.clockwise",
            IconSize::Small,
            theme.text.primary,
        ) {
            pill = pill.child(icon);
        }

        pill.child(
            div()
                .text_size(token.size)
                .line_height(token.line_height)
                .font(token.font)
                .text_color(theme.text.primary)
                .child(text.into()),
        )
    }

    /// Puts the refresh pill over content without disturbing it.
    pub fn over_content(
        theme: &AppTheme,
        content: impl IntoElement,
        pill: Option<AnyElement>,
    ) -> Div {
        let mut stack = div().relative().size_full().child(content);

        if let Some(pill) = pill {
            stack = stack.child(
                div()
                    .absolute()
                    .top(theme.spacing._400)
                    .left_0()
                    .right_0()
                    .flex()
                    .justify_center()
                    .child(pill),
            );
        }

        stack
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cached_page_keeps_its_rows_through_a_failed_refresh() {
        assert_eq!(
            PageStatus::Failed.presentation(true),
            PagePresentation::ContentWithRefreshError,
            "clearing the rows on a failed refresh is the defect this exists to prevent"
        );
    }

    #[test]
    fn a_cached_page_never_falls_back_to_the_initial_loader() {
        for status in [PageStatus::Idle, PageStatus::Loading, PageStatus::Loaded] {
            assert_eq!(status.presentation(true), PagePresentation::Content);
        }
    }

    #[test]
    fn an_empty_page_shows_its_loader_until_the_answer_arrives() {
        assert_eq!(
            PageStatus::Idle.presentation(false),
            PagePresentation::Loading
        );
        assert_eq!(
            PageStatus::Loading.presentation(false),
            PagePresentation::Loading
        );
    }

    #[test]
    fn a_page_that_loaded_nothing_says_so_rather_than_spinning_forever() {
        assert_eq!(
            PageStatus::Loaded.presentation(false),
            PagePresentation::Empty
        );
    }

    #[test]
    fn a_page_that_has_never_loaded_shows_the_blocking_error() {
        assert_eq!(
            PageStatus::Failed.presentation(false),
            PagePresentation::Error
        );
    }

    #[test]
    fn the_states_the_empty_and_error_views_name_have_glyphs() {
        for symbol in [
            "wifi.exclamationmark",
            "arrow.clockwise",
            "sparkles",
            "photo.on.rectangle.angled",
            "text.bubble",
            "bubble.left.and.bubble.right",
        ] {
            assert!(
                IconUi::asset_path(symbol).is_some(),
                "{symbol} has no glyph, so the state would draw a gap"
            );
        }
    }
}
