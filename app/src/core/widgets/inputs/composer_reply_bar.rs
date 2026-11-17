use std::rc::Rc;
use std::sync::Arc;

use gpui::{App, ElementId, FontWeight, Hsla, SharedString, Window};
use rise_platform::haptics::{ImpactFeedbackGenerator, ImpactStyle};
use rise_theme::{AppTheme, ThemeStore};
use rise_ui::IconUi;
use rise_ui::official_badge::OfficialBadge;
use rise_ui::rich_text::RichTextMarkup;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::rich_text::RichText;
use rise_ui::swiftui::text::TruncationMode;

use crate::core::ui::clever_image_ui::{CleverImageUi, ContentMode};
use crate::core::ui::formatted_text_ui::{FormattedText, FormattedTextOptions, FormattedTextStyle};
use crate::core::ui::premium_diamond::PremiumDiamond;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ComposerReplyBarKind {
    #[default]
    Reply,
    Edit,
}

/// row over the composer while a reply or edit is pending
/// caller adds and removes it with an opacity transition
#[derive(IntoElement)]
pub struct ComposerReplyBar {
    id: ElementId,
    pub kind: ComposerReplyBarKind,
    title: SharedString,
    subtitle: String,
    is_official: bool,
    is_premium: bool,
    media_thumbnail_source: Option<String>,
    theme: Option<AppTheme>,
    on_close: Rc<dyn Fn(&mut Window, &mut App)>,
}

impl ComposerReplyBar {
    pub fn new(
        id: impl Into<ElementId>,
        title: impl Into<SharedString>,
        subtitle: impl Into<String>,
        on_close: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            kind: ComposerReplyBarKind::Reply,
            title: title.into(),
            subtitle: subtitle.into(),
            is_official: false,
            is_premium: false,
            media_thumbnail_source: None,
            theme: None,
            on_close: Rc::new(on_close),
        }
    }

    pub fn kind(mut self, kind: ComposerReplyBarKind) -> Self {
        self.kind = kind;
        self
    }

    pub fn is_official(mut self, is_official: bool) -> Self {
        self.is_official = is_official;
        self
    }

    pub fn is_premium(mut self, is_premium: bool) -> Self {
        self.is_premium = is_premium;
        self
    }

    pub fn media_thumbnail_source(mut self, source: Option<String>) -> Self {
        self.media_thumbnail_source = source;
        self
    }

    pub fn theme(mut self, theme: Option<AppTheme>) -> Self {
        self.theme = theme;
        self
    }

    fn child_id(&self, name: &'static str) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static(name))
    }

    fn muted_style(theme: &AppTheme) -> FormattedTextStyle {
        let secondary = theme.text.secondary;
        let mut style = FormattedTextStyle::new(secondary, secondary, secondary);
        style.font_size = 13.5;
        style.code_foreground = Some(secondary);
        style.code_background = Some(secondary.opacity(0.14));
        style
    }

    fn one_line<E: Styled>(element: E) -> E {
        TruncationMode::Tail.apply(element.whitespace_nowrap().line_clamp(1))
    }
}

impl RenderOnce for ComposerReplyBar {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = match self.theme.clone() {
            Some(theme) => theme,
            None => ThemeStore::shared(cx).read(cx).current_theme().clone(),
        };
        let accent: Hsla = theme.primary._100;
        let secondary: Hsla = theme.text.secondary;

        let thumbnail = self
            .media_thumbnail_source
            .as_deref()
            .filter(|source| !source.is_empty())
            .map(|source| {
                div().flex_none().frame_size(32.0, 32.0).clip_shape(Shape::continuous(4.0)).child(
                    CleverImageUi::new(source.to_string())
                        .id(self.child_id("thumbnail"))
                        .content_mode(ContentMode::Fill)
                        .corner_radius(4.0)
                        .persist_to_disk(true)
                        .shows_progress_indicator(false)
                        .maximum_decode_pixel_dimension(96_i64)
                        .shows_failure_icon(false),
                )
            });

        let title_row = hstack(4.0)
            .w_full()
            .min_w_0()
            .child(Self::one_line(
                div()
                    .min_w_0()
                    .swiftui_font(Font::system_weight(13.5, FontWeight::SEMIBOLD))
                    .text_color(accent)
                    .child(self.title.clone()),
            ))
            .when(self.is_official, |row| row.child(OfficialBadge::new().size(px(13.0))))
            .when(self.is_premium, |row| row.child(PremiumDiamond::new().size(13.0)));

        let style = Self::muted_style(&theme);
        let options = FormattedTextOptions { no_url_subtitle: true, ..FormattedTextOptions::default() };
        let formatted = FormattedText::format(&RichTextMarkup::preview_line(&self.subtitle), &style, &options, &[]);
        let subtitle = Self::one_line(
            div()
                .w_full()
                .min_w_0()
                .swiftui_font(Font::system_size(style.font_size))
                .text_color(style.base_color)
                .child(RichText::new(self.child_id("subtitle"), formatted.attributed_body.clone())),
        );

        let texts = vstack(1.0)
            .align(HorizontalAlignment::Leading)
            .flex_1()
            .min_w_0()
            .child(title_row)
            .child(subtitle);

        let on_close = self.on_close.clone();
        let close = plain(self.child_id("close"))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .frame_size(30.0, 30.0)
            .circle()
            .on_press(move |window, cx| {
                ImpactFeedbackGenerator::new(ImpactStyle::Light).impact_occurred();
                on_close(window, cx);
            })
            .children(IconUi::sized("xmark", px(14.0), secondary, false));

        hstack(9.0)
            .w_full()
            .padding_edges(EdgeSet::VERTICAL, 6.0)
            .child(div().flex_none().frame_size(2.5, 30.0).continuous_corners(1.5).bg(accent))
            .children(thumbnail)
            .child(texts)
            .child(div().flex_none().w(px(8.0)))
            .child(close)
    }
}
